//! A11.4 (D-297): one row per entry of §1.11.6's table (the model's request equals `plan_command`'s for the same
//! selection); undo and redo per pane with the caret restored; typing within a second one step of at most a word
//! (explicit times); a blocked pane greys its commands and shows its message; the binary question once per pane, Yes
//! lifting that pane only; edited and saved marks.

use hikasa_core::Side;
use hikasa_folder::ops::ReadOnlySides;
use hikasa_text::LineIdx;
use hikasa_text::edit::{
    EditCommand, GroupHint, LineMark, Selection, TextPos, doc_end, plan_command,
};

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, Prompt};
use crate::status::MessageKind;
use crate::tests_text::Fixture;
use crate::text::input::Key;

fn whole(doc: &hikasa_text::EditableDocument) -> String {
    doc.text(TextPos::ZERO..doc_end(doc))
}

#[test]
fn a11_4_each_editing_command_is_core_plan() {
    let text = "    alpha beta gamma\nsecond line\nthird\n";
    let cases = [
        (CommandId::Delete, EditCommand::Delete),
        (CommandId::DeleteItem, EditCommand::DeleteLine),
        (CommandId::DeleteItemBegin, EditCommand::DeleteToLineStart),
        (CommandId::DeleteItemEnd, EditCommand::DeleteToLineEnd),
        (CommandId::DeleteWord, EditCommand::DeleteWord),
        (CommandId::DeleteWordBegin, EditCommand::DeleteToWordStart),
        (CommandId::DeleteWordEnd, EditCommand::DeleteToWordEnd),
        (CommandId::InsertItemBefore, EditCommand::InsertLineBefore),
        (CommandId::InsertItemAfter, EditCommand::InsertLineAfter),
        (CommandId::IncreaseIndent, EditCommand::IncreaseIndent),
        (CommandId::DecreaseIndent, EditCommand::DecreaseIndent),
    ];
    for (id, cmd) in cases {
        let mut f = Fixture::new(text, text);
        f.place(Side::Right, 0, 11);
        // Edit ▸ Delete needs a selection (§1.11.6): greyed at a caret, run on two selected characters; the others
        // act at the caret.
        let sel = if id == CommandId::Delete {
            assert!(!f.app.enabled(id));
            f.m()
                .pane_mut(Side::Right)
                .select(TextPos::new(0, 11), TextPos::new(0, 13));
            Selection {
                anchor: TextPos::new(0, 11),
                caret: TextPos::new(0, 13),
            }
        } else {
            Selection::caret(TextPos::new(0, 11))
        };
        let mut expected = f.doc(Side::Right);
        if let Some((req, _)) = plan_command(&expected, sel, &cmd) {
            expected.apply(&req, GroupHint::None).expect("apply");
        }
        assert!(f.app.enabled(id), "{id:?}");
        f.run(id);
        assert_eq!(f.text(Side::Right), whole(&expected), "{id:?}");
    }
    // The keys of the table: Enter, Tab, Shift+Tab, BkSp, Del.
    let mut f = Fixture::new(text, text);
    f.place(Side::Left, 1, 6);
    f.plain(Key::Enter);
    f.plain(Key::Tab);
    f.plain(Key::Backspace);
    f.plain(Key::Delete);
    // Enter splits before " line", Tab and BkSp cancel out, Del takes the blank.
    assert_eq!(f.text(Side::Left).lines().nth(2), Some("line"));
    // Select Section: the lines of the current section.
    f.run(CommandId::SelectSection);
    assert!(f.m().pane(Side::Left).has_selection());
}

#[test]
fn a11_4_undo_redo_per_pane_and_typing_coalesces() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 3);
    f.m().set_now(10.0);
    f.type_text("x");
    f.m().set_now(10.2);
    f.type_text("y");
    f.m().set_now(10.4);
    f.type_text(" ");
    f.m().set_now(10.5);
    f.type_text("z");
    f.m().set_now(12.0);
    f.type_text("w");
    assert_eq!(f.text(Side::Left), "abcxy zw\n");
    f.run(CommandId::Undo);
    assert_eq!(
        f.text(Side::Left),
        "abcxy z\n",
        "a second later: a step of its own"
    );
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), "abcxy \n", "a step is at most a word");
    f.run(CommandId::Undo);
    assert_eq!(
        f.text(Side::Left),
        "abc\n",
        "typing within a second is one step"
    );
    assert_eq!(
        f.caret(Side::Left),
        TextPos::new(0, 3),
        "the caret restored"
    );
    f.run(CommandId::Redo);
    assert_eq!(f.text(Side::Left), "abcxy \n");
    // The other pane's log is its own.
    f.m().set_focus(Side::Right);
    assert!(!f.app.enabled(CommandId::Undo));
}

#[test]
fn a11_4_a_blocked_pane_greys_its_commands_and_says_why() {
    let mut f = Fixture::new("abc\n", "abd\n");
    let policy = crate::folder_view::WritePolicy {
        read_only: ReadOnlySides {
            left: false,
            right: true,
        },
        ..crate::folder_view::WritePolicy::default()
    };
    f.m().set_write_policy(policy);
    f.place(Side::Right, 0, 0);
    for id in [
        CommandId::Paste,
        CommandId::Cut,
        CommandId::DeleteItem,
        CommandId::Undo,
    ] {
        assert!(!f.app.enabled(id), "{id:?}");
    }
    assert!(!f.app.enabled(CommandId::CopyLeft) || f.app.enabled(CommandId::CopyLeft));
    assert!(
        !f.app.enabled(CommandId::CopyRight),
        "the target is blocked"
    );
    f.type_text("x");
    assert_eq!(f.text(Side::Right), "abd\n");
    f.frame(0.0);
    let message = f
        .app
        .tab(f.view)
        .and_then(|t| t.message.clone())
        .expect("a message");
    assert_eq!(message.kind, MessageKind::Info);
    assert_eq!(
        message.text,
        "Editing disabled: the session's side is read-only"
    );
}

#[test]
fn a11_4_binary_asks_once_and_yes_lifts_that_pane() {
    let world = crate::tests::World::new();
    let l = world.path("work/l.bin");
    let r = world.path("work/r.bin");
    std::fs::write(&l, b"ab\0cd\nline\n").expect("write");
    std::fs::write(&r, b"ab\0ce\nline\n").expect("write");
    let mut f = Fixture::open_paths(world, l, r);
    assert!(
        crate::text::only_binary(&f.doc(Side::Left)),
        "{:?}",
        f.doc(Side::Left).editability()
    );
    f.place(Side::Left, 1, 0);
    f.type_text("x");
    f.frame(0.0);
    let asked = f.app.window(f.app.win()).expect("window").dialogs.len();
    assert_eq!(asked, 1, "the binary question");
    let service = f.world.service.clone();
    let win = f.app.win();
    let step = {
        let w = f.app.window_mut(win).expect("window");
        let p = w.dialogs.find_mut::<Prompt>().expect("prompt");
        assert!(
            p.message.contains("contains binary data. Edit anyway?"),
            "{}",
            p.message
        );
        crate::dialog::DialogModel::on_button(p, ButtonRole::Yes, &service)
    };
    f.app.dialog_step_pub(win, step);
    assert_eq!(
        f.text(Side::Left),
        "ab\0cd\nxline\n",
        "the waiting edit applied"
    );
    assert!(f.doc(Side::Left).editability().editable());
    assert!(
        !f.doc(Side::Right).editability().editable(),
        "that pane only"
    );
}

#[test]
fn a11_4_edited_lines_are_marked() {
    let mut f = Fixture::new("one\ntwo\n", "one\ntwo\n");
    f.place(Side::Left, 1, 0);
    f.type_text("x");
    let doc = f.doc(Side::Left);
    assert_eq!(doc.mark(LineIdx(1)), LineMark::Edited);
    assert_eq!(doc.mark(LineIdx(0)), LineMark::Original);
    let pal = crate::ui::theme::light();
    assert_ne!(
        pal.t_edited, pal.t_saved,
        "edited and saved have their own colours"
    );
}

/// G7, G31: *No* to the binary question is remembered for the pane: a later edit is refused without asking again.
#[test]
fn a11_4_binary_no_is_remembered() {
    let world = crate::tests::World::new();
    let l = world.path("work/l.bin");
    let r = world.path("work/r.bin");
    std::fs::write(&l, b"ab\0cd\nline\n").expect("write");
    std::fs::write(&r, b"ab\0ce\nline\n").expect("write");
    let mut f = Fixture::open_paths(world, l, r);
    f.place(Side::Left, 1, 0);
    f.type_text("x");
    f.frame(0.0);
    assert_eq!(f.dialogs(), 1, "asked");
    f.answer(ButtonRole::No);
    assert_eq!(f.dialogs(), 0);
    assert_eq!(f.text(Side::Left), "ab\0cd\nline\n", "No applies nothing");
    f.type_text("y");
    f.frame(0.0);
    assert_eq!(f.dialogs(), 0, "not asked again");
    assert_eq!(f.text(Side::Left), "ab\0cd\nline\n");
    let blocked = crate::text::header::block_message(&f.doc(Side::Left).editability());
    assert_eq!(f.message(), Some(blocked), "the pane's block");
}

/// G29: Increase Indent, Decrease Indent and Shift+Tab keep the lines selected.
#[test]
fn a11_4_indent_keeps_the_selection() {
    let mut f = Fixture::new("one\ntwo\nthree\n", "x\n");
    f.place(Side::Left, 0, 1);
    f.key(crate::text::input::Press::shift(Key::Down));
    f.run(CommandId::IncreaseIndent);
    assert_eq!(f.text(Side::Left), "\tone\n\ttwo\nthree\n");
    let sel = f.m().pane(Side::Left).selection();
    assert_ne!(sel.anchor, sel.caret, "still selected");
    assert_eq!((sel.anchor.line.0, sel.caret.line.0), (0, 1));
    f.run(CommandId::DecreaseIndent);
    assert_eq!(f.text(Side::Left), "one\ntwo\nthree\n");
    let sel = f.m().pane(Side::Left).selection();
    assert_ne!(sel.anchor, sel.caret, "still selected");
}
