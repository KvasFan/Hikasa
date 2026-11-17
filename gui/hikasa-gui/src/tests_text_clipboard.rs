//! A11.5 (D-298) and A11.6 (D-299): the clipboard of the text panes and the copies to the other side.
//!
//! Copy keeps CRLF and mixed terminators as stored; without a selection the caret's line goes; Cut is one step; Paste
//! of `a\r\nb\rc` into an LF pane writes the pane's breaks; files and images on the clipboard; the primary selection
//! written once a selection settles, a middle click pastes it; a focused find field keeps Ctrl+V. Each copy id with
//! selections and sections, one undo step on the target, a blocked target greys it, the gutter's buttons on each
//! difference section on screen, a button equal to Copy to Right of its section, the comparison exact after a copy.

use std::path::PathBuf;

use hikasa_core::Side;
use hikasa_session::clipboard::ClipboardPayload;
use hikasa_session::settings::Shortcut;
use hikasa_text::edit::TextPos;

use crate::commands::{CommandId, FocusArea};
use crate::tests_text::{Fixture, fresh_json};
use crate::text::input::{self, Key, Press};

fn last_payload(f: &Fixture) -> Option<String> {
    f.clip
        .payloads()
        .last()
        .map(|p: &ClipboardPayload| p.text.clone())
}

#[test]
fn a11_5_copy_keeps_the_stored_terminators_and_cut_is_one_step() {
    let left = "one\r\ntwo\nthree\rfour\n";
    let mut f = Fixture::new(left, "x\n");
    f.place(Side::Left, 0, 1);
    f.key(Press::shift(Key::Down));
    f.key(Press::shift(Key::Down));
    f.run(CommandId::Copy);
    f.settle();
    assert_eq!(
        last_payload(&f).as_deref(),
        Some("ne\r\ntwo\nt"),
        "the sticky x keeps column 1"
    );
    // No selection: the caret's line with its terminator.
    f.place(Side::Left, 0, 2);
    f.run(CommandId::Copy);
    f.settle();
    assert_eq!(last_payload(&f).as_deref(), Some("one\r\n"));
    // Cut: copied, then deleted as one step.
    f.place(Side::Left, 1, 0);
    f.key(Press::shift(Key::End));
    f.run(CommandId::Cut);
    f.settle();
    assert_eq!(last_payload(&f).as_deref(), Some("two"));
    assert_eq!(f.text(Side::Left), "one\r\n\nthree\rfour\n");
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), left);
}

#[test]
fn a11_5_paste_writes_the_panes_breaks_files_and_images() {
    let mut f = Fixture::new("abc\n", "x\n");
    f.place(Side::Left, 0, 3);
    f.clip.set_text("a\r\nb\rc");
    f.run(CommandId::Paste);
    f.settle();
    assert_eq!(f.text(Side::Left), "abca\nb\nc\n");
    // Files: their paths, one a line.
    let paths = vec![PathBuf::from("/x/one.txt"), PathBuf::from("/x/two.txt")];
    f.clip.set_files(paths);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::Paste);
    f.settle();
    assert!(
        f.text(Side::Left).starts_with("/x/one.txt\n/x/two.txt\n"),
        "{}",
        f.text(Side::Left)
    );
}

#[test]
fn a11_5_an_image_or_nothing_says_so() {
    let world = crate::tests::World::new();
    let l = world.write("work/a.txt", "abc\n");
    let r = world.write("work/b.txt", "abd\n");
    let mut f = Fixture::open_paths(world, l, r);
    let image = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::image(
        1,
        1,
        vec![0, 0, 0, 255],
    ));
    f.app
        .0
        .set_script_clipboard(std::sync::Arc::clone(&image) as _);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::Paste);
    f.settle();
    assert_eq!(f.text(Side::Left), "abc\n");
    assert_eq!(f.message().as_deref(), Some("The clipboard holds no text."));
}

#[test]
fn a11_5_the_primary_selection_settles_and_a_middle_click_pastes_it() {
    let mut f = Fixture::new("alpha beta\ngamma\n", "x\n");
    f.place(Side::Left, 0, 0);
    let before = f.clip.primaries().len();
    f.key(Press::shift(Key::End));
    // Not before 300 ms after the last Shift+move.
    f.frame(0.1);
    assert_eq!(f.clip.primaries().len(), before);
    f.frame(0.25);
    f.settle();
    assert_eq!(f.clip.primaries().len(), before + 1, "written once");
    assert_eq!(
        f.clip.primaries().last().map(String::as_str),
        Some("alpha beta")
    );
    f.settle();
    assert_eq!(f.clip.primaries().len(), before + 1, "not again");
    // A middle click moves the caret and pastes the primary selection.
    f.clip.set_primary("PRI");
    input::middle_click(f.m(), Side::Left, TextPos::new(1, 2));
    f.settle();
    assert_eq!(f.text(Side::Left), "alpha beta\ngaPRImma\n");
    // A focused text field keeps Ctrl+V (D-252).
    let key = Shortcut::parse("Ctrl+V").expect("key");
    assert_eq!(
        f.app.dispatch_key(f.app.win(), &key, FocusArea::TextField),
        None
    );
    assert_eq!(
        f.app.dispatch_key(f.app.win(), &key, FocusArea::Root),
        Some(CommandId::Paste)
    );
}

const LEFT: &str = "alpha 1\nbeta 2\ngamma 3\ndelta 4\nepsilon 5\nzeta 6\neta 7\ntheta 8\n";
const RIGHT: &str = "alpha 1\nbeta 20\ngamma 3\ndelta 4\nepsilon 50\nzeta 60\neta 7\ntheta 8\n";

/// LEFT with its lines `k` replaced.
fn with(base: &str, lines: &[(usize, &str)]) -> String {
    let mut v: Vec<&str> = base.lines().collect();
    for (k, t) in lines {
        v[*k] = t;
    }
    v.iter().map(|l| format!("{l}\n")).collect()
}

fn exact(f: &mut Fixture) {
    f.settle();
    let (l, r) = (f.text(Side::Left), f.text(Side::Right));
    assert_eq!(
        f.json(),
        fresh_json(&f.world, &l, &r),
        "the installed comparison is exact"
    );
}

#[test]
fn a11_6_each_copy_id_copies_rows_as_one_step() {
    // Copy to Right of the current section.
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Left, 1, 0);
    f.run(CommandId::CopyRight);
    exact(&mut f);
    assert_eq!(f.text(Side::Right), with(RIGHT, &[(1, "beta 2")]));
    f.m().set_focus(Side::Right);
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Right), RIGHT, "one step on the target");
    // Copy to Left of a selection's rows.
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Right, 4, 0);
    f.key(Press::shift(Key::Down));
    f.key(Press::shift(Key::Down));
    f.run(CommandId::CopyLeft);
    exact(&mut f);
    assert_eq!(
        f.text(Side::Left),
        with(LEFT, &[(4, "epsilon 50"), (5, "zeta 60")])
    );
    // Copy Line to Right: the cursor row only.
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Left, 4, 0);
    f.run(CommandId::CopyItemRight);
    exact(&mut f);
    assert_eq!(f.text(Side::Right), with(RIGHT, &[(4, "epsilon 5")]));
    // Copy to the other side: from the focus pane.
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Right, 1, 0);
    f.run(CommandId::CopySide);
    exact(&mut f);
    assert_eq!(f.text(Side::Left), with(LEFT, &[(1, "beta 20")]));
    f.place(Side::Right, 4, 0);
    f.run(CommandId::CopyItemSide);
    exact(&mut f);
    assert_eq!(
        f.text(Side::Left),
        with(LEFT, &[(1, "beta 20"), (4, "epsilon 50")])
    );
}

#[test]
fn a11_6_a_blocked_target_greys_the_copy() {
    let mut f = Fixture::new(LEFT, RIGHT);
    f.m().set_write_policy(crate::folder_view::WritePolicy {
        read_only: hikasa_folder::ops::ReadOnlySides {
            left: true,
            right: false,
        },
        ..crate::folder_view::WritePolicy::default()
    });
    f.place(Side::Right, 1, 0);
    assert!(!f.app.enabled(CommandId::CopyLeft));
    assert!(!f.app.enabled(CommandId::CopySide));
    assert!(f.app.enabled(CommandId::CopyRight));
}

#[test]
fn a11_6_the_gutter_buttons_copy_their_section() {
    let mut f = Fixture::new(LEFT, RIGHT);
    let rows = f.m().comparison().expect("cmp").row_count();
    let buttons = crate::text::edit::sections_starting_in(f.m(), 0..rows);
    assert_eq!(buttons.len(), 2, "one per difference section");
    for (start, section) in &buttons {
        assert_eq!(
            section.start, *start,
            "the button's rows are its own section"
        );
    }
    // A click on the second section's right arrow equals Copy to Right of that section.
    let (_, section) = buttons[1].clone();
    let mut g = Fixture::new(LEFT, RIGHT);
    let line = g.m().line_at_row(Side::Left, section.start).expect("line");
    g.place(Side::Left, line, 0);
    g.run(CommandId::CopyRight);
    g.settle();
    assert!(crate::text::edit::copy_rows_to(f.m(), Side::Left, section));
    exact(&mut f);
    assert_eq!(f.text(Side::Right), g.text(Side::Right));
}

#[test]
fn a11_6_after_a_copy_the_next_difference_follows_when_asked() {
    let world = crate::tests::World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[next_difference]\ngo_to_next_difference_after_copy = true\n",
    )]);
    let l = world.write("work/l.c", LEFT);
    let r = world.write("work/r.c", RIGHT);
    let mut f = Fixture::open_paths(world, l, r);
    f.place(Side::Left, 1, 0);
    let before = f.m().cursor().map(|r| r.0);
    f.run(CommandId::CopyRight);
    let after = f.m().cursor().map(|r| r.0);
    assert!(after > before, "{before:?} → {after:?}");
}

// ---- §1.11.35 W11a fixes and the review's gaps ------------------------------------------------------------------

/// F4: a Cut whose clipboard write fails keeps the text; a message says so.
#[test]
fn a11_5_a_cut_that_cannot_write_the_clipboard_keeps_the_text() {
    let mut f = Fixture::new("abc def\n", "abc def\n");
    let failing = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::failing(
        "no display",
    ));
    f.app
        .0
        .set_script_clipboard(std::sync::Arc::clone(&failing) as _);
    f.place(Side::Left, 0, 0);
    for _ in 0..3 {
        f.key(Press::shift(Key::Right));
    }
    f.run(CommandId::Cut);
    f.settle();
    assert_eq!(f.text(Side::Left), "abc def\n", "nothing was cut");
    assert!(!f.doc(Side::Left).is_modified());
    assert_eq!(
        f.message().as_deref(),
        Some(crate::text::clipboard::CUT_KEPT)
    );
}

/// F2: a Cut during a save deletes exactly what it copied, after the selection the user built before it; a selection
/// made after it (on the text shown) is replayed on the text after the Cut.
#[test]
fn a11_5_a_cut_during_a_save_deletes_what_it_copied() {
    use crate::text::input::Clicks;
    use crate::text::save::{self, SaveTarget};
    let mut f = Fixture::new("abc def\n", "abc def\n");
    f.place(Side::Left, 0, 0);
    f.type_text("1");
    input::press(f.m(), Side::Left, TextPos::new(0, 1), false, Clicks::One);
    input::press(f.m(), Side::Left, TextPos::new(0, 4), true, Clicks::One);
    let svc = f.world.service.clone();
    assert!(save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).is_some());
    f.run(CommandId::Cut);
    input::press(f.m(), Side::Left, TextPos::new(0, 5), false, Clicks::One);
    input::press(f.m(), Side::Left, TextPos::new(0, 8), true, Clicks::One);
    f.settle();
    assert_eq!(last_payload(&f).as_deref(), Some("abc"));
    assert_eq!(f.text(Side::Left), "1 def\n");
    let sel = f.m().pane(Side::Left).selection().range();
    assert_eq!(
        (sel.start, sel.end),
        (TextPos::new(0, 2), TextPos::new(0, 5)),
        "\"def\""
    );
    // One undo step brings the cut text back.
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), "1abc def\n");
}

/// F3: a gutter copy button whose target is being saved copies its section when the save ended.
#[test]
fn a11_6_a_gutter_copy_during_a_save_waits() {
    use crate::text::save::{self, SaveTarget};
    let mut f = Fixture::new(LEFT, RIGHT);
    let rows = f.m().comparison().expect("cmp").row_count();
    let buttons = crate::text::edit::sections_starting_in(f.m(), 0..rows);
    let svc = f.world.service.clone();
    f.place(Side::Right, 0, 0);
    assert!(save::begin(f.m(), &svc, vec![Side::Right], vec![SaveTarget::InPlace]).is_some());
    // Delete Line of the right's first line first: the button's section moved up a line meanwhile.
    f.run(CommandId::DeleteItem);
    assert!(!crate::text::edit::copy_section(
        f.m(),
        Side::Left,
        buttons[1].1.clone()
    ));
    exact(&mut f);
    let mut want: Vec<&str> = RIGHT.lines().collect();
    want[4] = "epsilon 5";
    want[5] = "zeta 6";
    want.remove(0);
    let want: String = want.iter().map(|l| format!("{l}\n")).collect();
    assert_eq!(f.text(Side::Right), want);
}

/// G8: a selection ending at a line's start leaves that line out of the copied rows; G9: the target's caret goes to
/// the copied lines' start.
#[test]
fn a11_6_selected_rows_and_the_targets_caret() {
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Left, 4, 0);
    f.key(Press::shift(Key::Down));
    f.run(CommandId::CopyRight);
    exact(&mut f);
    assert_eq!(
        f.text(Side::Right),
        with(RIGHT, &[(4, "epsilon 5")]),
        "zeta 60 stays"
    );
    assert_eq!(f.caret(Side::Right), TextPos::new(4, 0));
}

/// G14: the primary selection takes at most 1 MiB; G16: it settles 300 ms after the last Shift+move, not at once.
#[test]
fn a11_5_the_primary_selection_is_capped_and_waits() {
    let long = format!("{}\n", "é".repeat(600_000));
    let mut f = Fixture::new(&long, "x\n");
    f.place(Side::Left, 0, 0);
    f.key(Press::shift(Key::End));
    let now = f.m().now();
    let due = f.m().wake_at().expect("a wake-up for the settle");
    assert!(
        (due - (now + crate::text::clipboard::PRIMARY_SETTLE_S)).abs() < 1e-6,
        "{due} vs {now}"
    );
    f.frame(0.35);
    f.settle();
    let written = f.clip.primaries().last().cloned().expect("written");
    assert!(written.len() <= crate::text::clipboard::PRIMARY_MAX_BYTES);
    assert!(written.len() > crate::text::clipboard::PRIMARY_MAX_BYTES - 4);
    assert!(long.starts_with(&written));
}
