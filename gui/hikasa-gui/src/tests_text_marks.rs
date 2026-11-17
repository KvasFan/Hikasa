//! A11.17 (D-310): Ignored toggles and the session's keys with their checks; Align With by a click and by Enter, Esc
//! cancels, the toggle, Isolate, Clear, a dropped pair's notice; every conversion on a selection and on the file with
//! *Converted N lines*; Compare Selection to Clipboard with and without a selection (a new view, Save As only); Open
//! Clipboard into a modified pane asks first.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_session::clipboard::FixedClipboard;
use hikasa_text::edit::{TextPos, doc_end};
use hikasa_text::{Anchor, AnchorSide, Transform, transform};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::tests::World;
use crate::tests_text::Fixture;
use crate::text::input::{Clicks, Key};
use crate::text::marks;
use crate::window::ViewModel;

/// Sections at line 2 (changed) and 6 (the right's added line).
const L: &str = "a\nb\nc 1\nd\ne\nf\ng\n";
const R: &str = "a\nb\nc 2\nd\ne\nf\nnew\ng\n";

#[test]
fn a11_17_ignored_toggles_and_the_session_keeps_it() {
    let mut f = Fixture::new(L, R);
    // The caret in the first section (no selection): its lines on each side.
    f.place(Side::Left, 2, 0);
    f.run(CommandId::Ignored);
    let ignored = f.m().live_view().expect("live").live.ignored().clone();
    assert!(ignored.left.contains(2) && ignored.right.contains(2));
    assert_eq!(f.m().checked(CommandId::Ignored), Some(true));
    assert_eq!(f.m().comparison().expect("cmp").stats().ignored_rows, 1);
    // No recompare: the comparison is not provisional.
    assert!(!f.m().comparison().expect("cmp").provisional());
    // The session of the view holds the lines and their checks (D-282, through core's `session_for_view`).
    let (request, _) = f.m().duplicate();
    let (session, _) = f
        .world
        .service
        .session_for_view(&request, Default::default(), Default::default(), None)
        .expect("session");
    assert_eq!(session.specs.ignored_left, vec!["3".to_owned()]);
    assert_eq!(session.specs.ignored_right, vec!["3".to_owned()]);
    let doc = f.doc(Side::Left);
    assert_eq!(
        session.specs.ignored_check_left,
        format!(
            "{:08x}",
            hikasa_text::ignored_check(doc.document(), &ignored.left).expect("check")
        )
    );
    // Again: every line is ignored already, so they are removed.
    f.run(CommandId::Ignored);
    assert!(f.m().live_view().expect("live").live.ignored().is_empty());
    assert_eq!(f.m().checked(CommandId::Ignored), Some(false));
}

#[test]
fn a11_17_align_with_by_click_and_by_enter() {
    let mut f = Fixture::new(L, R);
    // Left line 4 ("e") with right line 6 ("new") by a click.
    f.place(Side::Left, 4, 0);
    f.run(CommandId::AlignWith);
    assert!(f.m().align.is_some());
    assert_eq!(f.message().as_deref(), Some(marks::ALIGN_STATUS));
    // A click in the same pane does not complete it.
    assert!(!marks::align_click(f.m(), Side::Left, 2));
    assert!(f.m().align.is_some() && marks::anchors(f.m()).is_empty());
    crate::text::input::press(f.m(), Side::Right, TextPos::new(6, 0), false, Clicks::One);
    assert!(f.m().align.is_none());
    assert_eq!(
        marks::anchors(f.m()),
        vec![Anchor::Pair {
            left: 4..5,
            right: 6..7
        }]
    );
    f.settle();
    // The next recompare ran in full with the pair: those lines share a row.
    let cmp = Arc::clone(f.m().comparison().expect("cmp"));
    let row = cmp
        .row_for_line(Side::Left, hikasa_text::LineIdx(4))
        .expect("row");
    assert_eq!(cmp.row(row).and_then(|r| r.right).map(|l| l.0), Some(6));
    assert_eq!(marks::anchor_mark(f.m(), row.0), Some(true));
    // Align With on lines inside the pair removes it (the toggle).
    f.place(Side::Left, 4, 0);
    f.run(CommandId::AlignWith);
    assert!(marks::anchors(f.m()).is_empty());
    // By a selection and Enter in the other pane; Esc cancels.
    f.place(Side::Left, 0, 0);
    f.run(CommandId::AlignWith);
    f.plain(Key::Escape);
    assert!(f.m().align.is_none() && marks::anchors(f.m()).is_empty());
    f.place(Side::Left, 0, 0);
    f.run(CommandId::AlignWith);
    f.m().set_focus(Side::Right);
    f.m()
        .pane_mut(Side::Right)
        .select(TextPos::new(0, 0), TextPos::new(2, 0));
    f.plain(Key::Enter);
    assert_eq!(
        marks::anchors(f.m()),
        vec![Anchor::Pair {
            left: 0..1,
            right: 0..2
        }]
    );
    assert_eq!(
        f.text(Side::Right),
        R,
        "Enter completed the mode, no new line"
    );
    // Clear.
    assert!(f.app.enabled(CommandId::AnchorsClear));
    f.run(CommandId::AnchorsClear);
    assert!(marks::anchors(f.m()).is_empty());
}

#[test]
fn a11_17_isolate_and_the_dropped_pairs_notice() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Right, 6, 0);
    f.run(CommandId::Isolate);
    assert_eq!(
        marks::anchors(f.m()),
        vec![Anchor::Isolate {
            side: AnchorSide::Right,
            lines: 6..7
        }]
    );
    f.settle();
    let row = f.m().row_of_line(Side::Right, 6).expect("row");
    assert_eq!(marks::anchor_mark(f.m(), row), Some(false));
    f.run(CommandId::Isolate);
    assert!(marks::anchors(f.m()).is_empty());
    // A pair whose lines were all deleted is dropped by core: a warning notice.
    f.place(Side::Left, 5, 0);
    f.run(CommandId::AlignWith);
    crate::text::input::press(f.m(), Side::Right, TextPos::new(5, 0), false, Clicks::One);
    f.settle();
    f.place(Side::Left, 5, 0);
    f.run(CommandId::DeleteItem);
    f.settle();
    let win = f.app.win();
    let notices: Vec<String> = f
        .app
        .window(win)
        .map(|w| w.notices.all().iter().map(|n| n.text.clone()).collect())
        .unwrap_or_default();
    assert!(
        notices
            .iter()
            .any(|n| n.contains("anchor") || n.contains("align")),
        "{notices:?}"
    );
}

#[test]
fn a11_17_conversions_equal_core() {
    let text = "a  \n    b\t\nc\r\n\td \n";
    let cases = [
        (CommandId::TrimTrailWS, Transform::TrimTrailing),
        (CommandId::LeadSpaceToTab, Transform::LeadingSpacesToTabs),
        (CommandId::TabToSpace, Transform::TabsToSpaces),
        (
            CommandId::ToWindows,
            Transform::LineEndings(hikasa_text::LineEnding::CrLf),
        ),
        (
            CommandId::ToUnix,
            Transform::LineEndings(hikasa_text::LineEnding::Lf),
        ),
        (
            CommandId::ToMac,
            Transform::LineEndings(hikasa_text::LineEnding::Cr),
        ),
    ];
    for (id, t) in cases {
        // The file.
        let mut f = Fixture::new(text, text);
        f.place(Side::Left, 0, 0);
        let mut expected = f.doc(Side::Left);
        let tab = expected.options().tab_size;
        let out = transform(&expected, t, None, tab);
        if let Some(req) = &out.request {
            expected
                .apply(req, hikasa_text::edit::GroupHint::None)
                .expect("apply");
        }
        f.run(id);
        assert_eq!(
            f.text(Side::Left),
            expected.text(TextPos::ZERO..doc_end(&expected)),
            "{id:?}"
        );
        f.settle();
        assert_eq!(
            f.message(),
            Some(marks::converted_text(out.lines)),
            "{id:?}"
        );
        if out.request.is_some() {
            assert_eq!(f.doc(Side::Left).undo_steps(), 1, "{id:?}: one step");
        }
        // A selection that spans lines: those lines only.
        let mut f = Fixture::new(text, text);
        f.m().set_focus(Side::Right);
        f.m()
            .pane_mut(Side::Right)
            .select(TextPos::new(1, 0), TextPos::new(3, 0));
        let mut expected = f.doc(Side::Right);
        let out = transform(&expected, t, Some(1..3), tab);
        if let Some(req) = &out.request {
            expected
                .apply(req, hikasa_text::edit::GroupHint::None)
                .expect("apply");
        }
        f.run(id);
        assert_eq!(
            f.text(Side::Right),
            expected.text(TextPos::ZERO..doc_end(&expected)),
            "{id:?} on a selection"
        );
    }
    assert_eq!(marks::converted_text(0), "No line needed converting.");
}

#[test]
fn a11_17_compare_selection_to_clipboard() {
    let mut f = Fixture::new(L, R);
    f.clip.set_text("b\nX\n");
    f.m().set_focus(Side::Left);
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(1, 0), TextPos::new(3, 0));
    let before = f.app.tabs().len();
    f.run(CommandId::CompareToClipboard);
    f.settle();
    assert_eq!(f.app.tabs().len(), before + 1, "a new view");
    let view = f.app.tabs().iter().map(|t| t.id).max().expect("tab");
    let (left, right, pair) = match f.app.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Text(m)) => (
            m.doc(Side::Left).map(|d| d.text(TextPos::ZERO..doc_end(d))),
            m.doc(Side::Right)
                .map(|d| d.text(TextPos::ZERO..doc_end(d))),
            (
                m.is_text_pair(),
                m.doc(Side::Left).is_some_and(|d| d.location().is_none()),
            ),
        ),
        _ => panic!("a text view"),
    };
    assert_eq!(left.as_deref(), Some("b\nc 1\n"));
    assert_eq!(right.as_deref(), Some("b\nX\n"));
    assert_eq!(
        pair,
        (true, true),
        "a text pair: no location, Save is Save As"
    );
    // Without a selection: the whole document.
    f.app.activate(0);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::CompareToClipboard);
    f.settle();
    let view = f.app.tabs().iter().map(|t| t.id).max().expect("tab");
    let left = match f.app.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Text(m)) => m.doc(Side::Left).map(|d| d.text(TextPos::ZERO..doc_end(d))),
        _ => None,
    };
    assert_eq!(left.as_deref(), Some(L));
}

#[test]
fn a11_17_open_clipboard_into_a_modified_pane_asks_first() {
    let clip = Arc::new(FixedClipboard::text("from the clipboard\n"));
    let provider = Arc::clone(&clip);
    let world = World::with_init(move |init| init.clipboard = provider);
    let mut f = Fixture::in_world(world, L, R);
    f.place(Side::Right, 0, 0);
    f.type_text("edit ");
    f.run(CommandId::OpenClipboard);
    f.settle();
    let asked = f.prompt().expect("D-301's question");
    assert!(asked.contains("right.c"), "{asked}");
    f.answer(ButtonRole::No);
    f.settle();
    assert_eq!(f.text(Side::Right), "from the clipboard\n");
    assert_eq!(f.text(Side::Left), L, "the other pane kept");
    // A clipboard without text: the message panel.
    clip.set_files(vec![std::path::PathBuf::from("/x")]);
    f.run(CommandId::OpenClipboard);
    f.settle();
    assert!(
        f.message()
            .is_some_and(|m| m.starts_with(crate::text::clipboard::NO_TEXT)),
        "{:?}",
        f.message()
    );
}

/// A11.21's GUI part (D-314): the command line's words become the view's formats, anchors and ignored lines (core's
/// parsers through `open_view_request`); a word beyond a document is a warning notice and is dropped, the view opens.
#[test]
fn a11_21_the_words_become_the_views() {
    let world = World::new();
    let l = world.write("work/l.c", L);
    let r = world.write("work/r.c", R);
    let mut app = world.app(None);
    let options = hikasa_session::ipc::OpenOptions {
        formats: [None, Some("Everything Else".into())],
        anchors: vec!["5:7".into(), "left:99".into()],
        ignored: vec!["right:3".into()],
        ..hikasa_session::ipc::OpenOptions::default()
    };
    let specs = vec![l.display().to_string(), r.display().to_string()];
    let request = world
        .service
        .open_view_request(hikasa_session::ipc::ViewKind::TextCompare, &specs, &options)
        .expect("a request");
    let target = app.open_target();
    let view = app.0.open_local(
        hikasa_session::ipc::ViewKind::TextCompare,
        request,
        &options,
        specs,
        target,
    );
    world.settle(&mut app);
    let m = crate::tests::text_model(&mut app, view);
    assert_eq!(
        marks::anchors(m),
        vec![Anchor::Pair {
            left: 4..5,
            right: 6..7
        }]
    );
    let ignored = m.live_view().expect("live").live.ignored().clone();
    assert!(ignored.right.contains(2) && ignored.left.is_empty());
    let response = m.response().expect("response");
    assert_eq!(
        response.right_format.as_ref().map(|f| f.name.as_str()),
        Some("Everything Else")
    );
    let win = app.win();
    let notices: Vec<String> = app
        .window(win)
        .map(|w| w.notices.all().iter().map(|n| n.text.clone()).collect())
        .unwrap_or_default();
    assert!(
        notices.iter().any(|n| n.contains("beyond the end")),
        "{notices:?}"
    );
}
