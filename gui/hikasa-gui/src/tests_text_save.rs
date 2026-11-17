//! A11.7 (D-300) and A11.8 (D-301): saves in the window and the unsaved-edits prompts.
//!
//! Save of each pane, Save All as one plan, Save As through the browse dialog's answer (the location moves), the
//! questions through the prompts (Yes, Cancel, *Save as UTF-8*), the bytes equal `text_save_*` on the same document,
//! the marks and `is_modified` after, a failed item leaving the document modified, keys typed during a save applied
//! after it in order (and refused past 1,024). Every close of a modified view asks once per document: Save, Discard,
//! Cancel, Save All, Discard All; a failed save cancels the close.

use hikasa_core::{JobKind, Side};
use hikasa_session::api::{SaveAnswers, apply_save_report};
use hikasa_text::LineIdx;
use hikasa_text::edit::LineMark;

use crate::browse::BrowseTarget;
use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::tests_text::Fixture;
use crate::text::save::{self, SaveTarget};

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

#[test]
fn a11_7_save_writes_what_core_writes_and_marks_the_lines_saved() {
    let mut f = Fixture::new("one\r\ntwo\r\n", "one\ntwo\n");
    f.place(Side::Left, 1, 3);
    f.type_text("!");
    // What core writes for the same document (a plan on a copy, no question).
    let svc = f.world.service.clone();
    let doc = f.doc(Side::Left);
    let req = svc
        .live_save_request(f.m().live_view().expect("live"), Side::Left)
        .expect("request");
    let job = svc.job(
        JobKind::TextSave,
        std::sync::Arc::new(hikasa_core::NullProgress),
    );
    let plan = svc
        .text_save_plan(&[(Some(Side::Left), &doc, &req)], &job)
        .expect("plan");
    let summary = format!("{:?}", plan.items[0].content);
    drop(plan);
    assert!(f.app.enabled(CommandId::Save));
    f.run(CommandId::Save);
    f.settle();
    assert_eq!(read(&f.paths[0]), b"one\r\ntwo!\r\n", "{summary}");
    let doc = f.doc(Side::Left);
    assert!(!doc.is_modified());
    assert_eq!(doc.mark(LineIdx(1)), LineMark::SavedEdit);
    assert!(!f.app.enabled(CommandId::Save), "nothing left to save");
    let _ = (SaveAnswers::default(), apply_save_report);
}

#[test]
fn a11_7_save_all_is_one_plan_and_save_as_moves_the_location() {
    let mut f = Fixture::new("left\n", "right\n");
    f.place(Side::Left, 0, 0);
    f.type_text("L");
    f.place(Side::Right, 0, 0);
    f.type_text("R");
    f.run(CommandId::SaveAll);
    f.settle();
    assert_eq!(read(&f.paths[0]), b"Lleft\n");
    assert_eq!(read(&f.paths[1]), b"Rright\n");
    // Save As: the browse dialog's answer saves there; the document's location, the header and the request follow.
    f.type_text("2");
    f.run(CommandId::SaveAs);
    assert_eq!(f.dialogs(), 1, "the browse dialog");
    let target = f.world.path("work/copy.c");
    f.browse_answer(
        BrowseTarget::TextSaveAs {
            view: f.view,
            side: Side::Right,
            local: false,
        },
        Some(&target),
    );
    assert_eq!(read(&target), b"R2right\n");
    assert_eq!(read(&f.paths[1]), b"Rright\n", "the old file stays");
    let doc = f.doc(Side::Right);
    let location = doc
        .location()
        .map(hikasa_core::Spec::canonical)
        .unwrap_or_default();
    assert!(location.ends_with("copy.c"), "{location}");
    let title = crate::text::header::title(f.m(), Side::Right);
    assert!(title.ends_with("copy.c"), "{title}");
    assert!(f.m().request().right.to_string_lossy().ends_with("copy.c"));
}

#[test]
fn a11_7_questions_are_asked_through_prompts() {
    let mut f = Fixture::new("abc\n", "abc\n");
    // Unmappable characters: windows-1252 cannot write 日; Save as UTF-8 writes UTF-8.
    assert!(crate::text::header::save_with_encoding(
        f.m(),
        Side::Left,
        "windows-1252"
    ));
    f.place(Side::Left, 0, 3);
    f.type_text("日");
    f.run(CommandId::Save);
    f.settle();
    let question = f.prompt().expect("a question");
    assert!(
        f.prompt_buttons().contains(&"Save as UTF-8".to_owned()),
        "{question}"
    );
    f.answer(ButtonRole::Cancel);
    assert_eq!(read(&f.paths[0]), b"abc\n", "Cancel writes nothing");
    assert!(f.doc(Side::Left).is_modified());
    f.run(CommandId::Save);
    f.settle();
    f.answer(crate::app::text::SAVE_UTF8);
    assert_eq!(read(&f.paths[0]), "abc日\n".as_bytes());
    assert!(!f.doc(Side::Left).is_modified());
}

#[cfg(unix)]
#[test]
fn a11_7_a_read_only_target_asks_and_yes_writes() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new("abc\n", "abc\n");
    std::fs::set_permissions(&f.paths[0], std::fs::Permissions::from_mode(0o444)).expect("chmod");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::Save);
    f.settle();
    if f.prompt().is_none() {
        // Running as a user who may write anything (root): no question to ask.
        return;
    }
    f.answer(ButtonRole::Yes);
    assert_eq!(read(&f.paths[0]), b"xabc\n");
}

#[test]
fn a11_7_a_failed_item_leaves_the_document_modified() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    let missing = f.world.path("work/no/such/dir/x.c");
    f.run(CommandId::SaveAs);
    f.browse_answer(
        BrowseTarget::TextSaveAs {
            view: f.view,
            side: Side::Left,
            local: false,
        },
        Some(&missing),
    );
    assert!(f.doc(Side::Left).is_modified() || missing.exists());
}

#[test]
fn a11_7_keys_during_a_save_wait_and_apply_in_order() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 3);
    f.type_text("1");
    let svc = f.world.service.clone();
    assert!(save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).is_some());
    // Frozen until the save lands: queued, not applied.
    for c in ["2", "3", "4"] {
        f.type_text(c);
    }
    assert_eq!(f.text(Side::Left), "abc1\n");
    assert_eq!(crate::text::input::queued(f.m()), 3);
    f.settle();
    assert_eq!(
        read(&f.paths[0]),
        b"abc1\n",
        "the save wrote the text of its start"
    );
    assert_eq!(f.text(Side::Left), "abc1234\n", "then the keys, in order");
    assert!(f.doc(Side::Left).is_modified());
    // Past 1,024 queued inputs the rest is refused.
    assert!(save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).is_some());
    for _ in 0..1_030 {
        f.type_text("z");
    }
    assert_eq!(crate::text::input::queued(f.m()), crate::text::MAX_QUEUED);
    f.settle();
    assert_eq!(
        f.message().as_deref(),
        Some(crate::text::input::WAIT_FOR_SAVE)
    );
    assert_eq!(
        f.text(Side::Left).matches('z').count(),
        crate::text::MAX_QUEUED
    );
}

// ---- A11.8: unsaved edits at close ---------------------------------------------------------------------------------

#[test]
fn a11_8_close_asks_once_per_document() {
    // Discard: the tab closes, the file stays.
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::CloseTab);
    let q = f.prompt().expect("the prompt");
    assert_eq!(q, "Save changes to left.c?");
    f.answer(ButtonRole::No);
    assert!(f.app.tab(f.view).is_none(), "closed");
    assert_eq!(read(&f.paths[0]), b"abc\n");

    // Cancel: the tab and its edits stay.
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::CloseTab);
    f.answer(ButtonRole::Cancel);
    assert!(f.app.tab(f.view).is_some());
    assert!(f.doc(Side::Left).is_modified());

    // Save: written, then closed.
    f.run(CommandId::CloseTab);
    f.answer(ButtonRole::Yes);
    assert_eq!(read(&f.paths[0]), b"xabc\n");
    assert!(f.app.tab(f.view).is_none());
}

#[test]
fn a11_8_save_all_and_discard_all_answer_every_document() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.place(Side::Right, 0, 0);
    f.type_text("y");
    f.run(CommandId::CloseTab);
    assert!(f.prompt_buttons().contains(&"Discard All".to_owned()));
    f.answer(crate::app::text::DISCARD_ALL);
    assert!(f.prompt().is_none(), "the second document is not asked");
    assert!(f.app.tab(f.view).is_none());

    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.place(Side::Right, 0, 0);
    f.type_text("y");
    f.run(CommandId::CloseTab);
    f.answer(crate::app::text::SAVE_ALL);
    assert_eq!(read(&f.paths[0]), b"xabc\n");
    assert_eq!(read(&f.paths[1]), b"yabc\n");
    assert!(f.app.tab(f.view).is_none());
}

#[test]
fn a11_8_a_failed_save_cancels_the_close() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    // The file's folder goes away: the in-place save cannot write.
    let dir = f.paths[0].parent().expect("dir").to_path_buf();
    let moved = dir.with_file_name("work-gone");
    std::fs::rename(&dir, &moved).expect("rename");
    f.run(CommandId::CloseTab);
    f.answer(ButtonRole::Yes);
    std::fs::rename(&moved, &dir).expect("rename back");
    assert!(f.app.tab(f.view).is_some(), "the close was cancelled");
    assert!(f.doc(Side::Left).is_modified());
}

#[test]
fn a11_8_reload_of_a_modified_view_asks_first() {
    let mut f = Fixture::new("abc\n", "abd\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::Reload);
    assert!(f.prompt().is_some_and(|p| p.starts_with("Save changes to")));
    f.answer(ButtonRole::No);
    assert_eq!(f.text(Side::Left), "abc\n", "reloaded: the edits dropped");
}

/// A11.30: F1 in the W11a prompts opens their topics.
#[test]
fn a11_30_the_prompts_open_their_topics() {
    for id in [
        crate::app::text::UNSAVED_TOPIC,
        crate::app::text::SAVE_TOPIC,
    ] {
        assert!(hikasa_session::help::topic(id).is_some(), "{id}");
    }
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::CloseTab);
    let win = f.app.win();
    let topic = f
        .app
        .window_mut(win)
        .and_then(|w| w.dialogs.top_mut())
        .map(|d| d.help_topic());
    assert_eq!(topic, Some(crate::app::text::UNSAVED_TOPIC));
    f.answer(ButtonRole::Cancel);
}

// ---- §1.11.35 W11a fixes (the adversarial review's findings) ----------------------------------------------------

fn text_of(path: &std::path::Path) -> String {
    String::from_utf8(read(path)).expect("utf8")
}

/// A Text Compare view whose left side is `clipboard://` (no place to save in) and whose right side is a file.
pub(crate) fn clipboard_fixture(right: &str) -> Fixture {
    let fixed = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text(
        "clip text\n",
    ));
    let provider = std::sync::Arc::clone(&fixed);
    let world = crate::tests::World::with_init(move |init| init.clipboard = provider);
    let r = world.write("work/right.c", right);
    Fixture::open_paths(world, std::path::PathBuf::from("clipboard://"), r)
}

/// F1: each save has an id; a plain save leaves no result that a later waiter could take for its own.
#[test]
fn a11_7_a_save_result_belongs_to_its_save() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    let svc = f.world.service.clone();
    let id = save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).expect("a save");
    f.settle();
    assert_eq!(
        save::take_ended(f.m(), id + 1),
        None,
        "another save's result"
    );
    assert_eq!(save::take_ended(f.m(), id), Some(save::Ended::Saved));
    assert_eq!(save::take_ended(f.m(), id), None, "taken once");
    // A plain Ctrl+S: its result is dropped when the next save starts.
    f.type_text("y");
    f.run(CommandId::Save);
    f.settle();
    f.type_text("z");
    let next =
        save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).expect("a save");
    assert!(next > id);
    assert_eq!(save::take_ended(f.m(), id), None);
    f.settle();
    assert_eq!(save::take_ended(f.m(), next), Some(save::Ended::Saved));
}

/// F1: a close's *Save* after an earlier Ctrl+S waits for its own save: the save's question (the file changed on disk)
/// is asked and the tab stays until it is answered; Cancel keeps the tab and the edits.
#[test]
fn a11_8_a_close_waits_for_its_own_save_after_an_earlier_one() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::Save);
    f.settle();
    assert_eq!(text_of(&f.paths[0]), "xabc\n");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&f.paths[0], "other program\n").expect("write");
    f.type_text("y");
    f.run(CommandId::CloseTab);
    assert_eq!(f.prompt().as_deref(), Some("Save changes to left.c?"));
    f.answer(ButtonRole::Yes);
    assert!(f.app.tab(f.view).is_some(), "the view waits for its save");
    let question = f
        .prompt()
        .expect("the save's question about the file changed on disk");
    f.answer(ButtonRole::Cancel);
    assert!(
        f.app.tab(f.view).is_some(),
        "a cancelled save cancels the close ({question})"
    );
    assert_eq!(f.text(Side::Left), "xyabc\n");
    assert_eq!(text_of(&f.paths[0]), "other program\n");
}

/// F1: Reload's *Save* after an earlier Ctrl+S reloads only after its own save wrote.
#[test]
fn a11_8_reload_saves_first_after_an_earlier_save() {
    let mut f = Fixture::new("abc\n", "abd\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::Save);
    f.settle();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&f.paths[0], "other program\n").expect("write");
    f.type_text("y");
    f.run(CommandId::Reload);
    assert!(f.prompt().is_some_and(|p| p.starts_with("Save changes to")));
    f.answer(ButtonRole::Yes);
    assert!(f.prompt().is_some(), "the save's question first");
    assert_eq!(f.text(Side::Left), "xyabc\n", "nothing reloaded yet");
    f.answer(ButtonRole::Yes);
    assert_eq!(text_of(&f.paths[0]), "xyabc\n");
    assert_eq!(f.text(Side::Left), "xyabc\n", "reloaded what was saved");
    assert!(!f.doc(Side::Left).is_modified());
}

/// F11: a close's Save As answer that starts no save ends the close (with a message); a later close asks again.
#[test]
fn a11_8_a_save_as_answer_that_starts_no_save_cancels_the_close() {
    let mut f = clipboard_fixture("abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("L");
    let view = f.view;
    for answer in ["nosuch://x", "cancel"] {
        f.run(CommandId::CloseTab);
        assert!(f.prompt().is_some(), "{answer}: the prompt");
        f.answer(ButtonRole::Yes);
        assert_eq!(f.dialogs(), 1, "the Save As browse dialog");
        let path = std::path::PathBuf::from(answer);
        f.browse_answer(
            BrowseTarget::TextSaveAs {
                view,
                side: Side::Left,
                local: false,
            },
            (answer != "cancel").then_some(path.as_path()),
        );
        assert!(f.app.tab(view).is_some(), "{answer}");
        let msg = f.message().unwrap_or_default();
        assert!(msg.contains("close was cancelled"), "{answer}: {msg}");
    }
    // Save on File System As refuses a place that is not a file: the close ends too.
    f.run(CommandId::CloseTab);
    f.answer(ButtonRole::Yes);
    f.browse_answer(
        BrowseTarget::TextSaveAs {
            view,
            side: Side::Left,
            local: true,
        },
        Some(std::path::Path::new("clipboard://")),
    );
    assert!(f.app.tab(view).is_some());
    assert!(f.message().is_some_and(|m| m.contains("file system only")));
    // The close is not wedged: the next one asks and Discard closes.
    f.run(CommandId::CloseTab);
    assert!(f.prompt().is_some());
    f.answer(ButtonRole::No);
    assert!(f.app.tab(view).is_none());
}

/// F12: Save All saves the panes with a place in one plan, then asks Save As for a modified pane without one.
#[test]
fn a11_7_save_all_asks_save_as_for_a_pane_without_place() {
    let mut f = clipboard_fixture("abc\n");
    assert!(f.doc(Side::Left).location().is_none());
    f.place(Side::Left, 0, 0);
    f.type_text("L");
    f.place(Side::Right, 0, 0);
    f.type_text("R");
    f.run(CommandId::SaveAll);
    f.settle();
    assert_eq!(text_of(&f.paths[1]), "Rabc\n");
    assert_eq!(f.dialogs(), 1, "Save As for the clipboard pane");
    let target = f.world.path("work/from-clipboard.c");
    let view = f.view;
    f.browse_answer(
        BrowseTarget::TextSaveAs {
            view,
            side: Side::Left,
            local: false,
        },
        Some(&target),
    );
    assert_eq!(text_of(&target), "Lclip text\n");
    assert!(!f.doc(Side::Left).is_modified());
    assert!(!f.doc(Side::Right).is_modified());
}

/// F12: Reload's *Save* with a pane without a place: its Save As cancelled cancels the reload (the edits stay).
#[test]
fn a11_8_reload_save_with_a_cancelled_save_as_reloads_nothing() {
    let mut f = clipboard_fixture("abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("L");
    f.place(Side::Right, 0, 0);
    f.type_text("R");
    f.run(CommandId::Reload);
    f.answer(ButtonRole::Yes);
    assert_eq!(text_of(&f.paths[1]), "Rabc\n", "the right pane saved first");
    assert_eq!(f.dialogs(), 1, "then Save As of the left one");
    let view = f.view;
    f.browse_answer(
        BrowseTarget::TextSaveAs {
            view,
            side: Side::Left,
            local: false,
        },
        None,
    );
    assert_eq!(
        f.text(Side::Left),
        "Lclip text\n",
        "not reloaded: the edits stay"
    );
    assert!(f.doc(Side::Left).is_modified());
}

/// F2/F3: every input of a frozen pane waits in order — clicks too: keys typed at two places during a save land at
/// those places.
#[test]
fn a11_7_clicks_and_keys_during_a_save_replay_in_order() {
    use crate::text::input::{self, Clicks};
    use hikasa_text::edit::TextPos;
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 3);
    f.type_text("!");
    let svc = f.world.service.clone();
    assert!(save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).is_some());
    input::press(f.m(), Side::Left, TextPos::new(0, 0), false, Clicks::One);
    f.type_text("1");
    // The click at the end of the line shown ("abc!"): after the "!" also once "1" went in before it.
    input::press(f.m(), Side::Left, TextPos::new(0, 4), false, Clicks::One);
    f.type_text("2");
    assert_eq!(
        f.caret(Side::Left),
        TextPos::new(0, 4),
        "the pane shows nothing yet"
    );
    f.settle();
    assert_eq!(f.text(Side::Left), "1abc!2\n");
    assert_eq!(text_of(&f.paths[0]), "abc!\n");
}

/// F3: a copy to a frozen pane is planned at its turn: after the Delete Line queued before it, on the row the caret
/// went to — the same as the same inputs without a save (the click mapped to the line it named).
#[test]
fn a11_7_a_queued_copy_is_planned_at_its_turn() {
    use crate::text::input::{self, Clicks};
    use hikasa_text::edit::TextPos;
    let (left, right) = ("a\nB\nc\nd\n", "a\nb\nc\nD\n");
    // Without a save.
    let mut g = Fixture::new(left, right);
    g.place(Side::Right, 0, 0);
    g.run(CommandId::DeleteItem);
    input::press(g.m(), Side::Right, TextPos::new(2, 0), false, Clicks::One);
    g.run(CommandId::CopyRight);
    g.settle();
    let want = g.text(Side::Right);
    assert_eq!(want, "b\nc\nd\n", "the d/D section copied");
    // During a save: queued, then the same.
    let mut f = Fixture::new(left, right);
    let svc = f.world.service.clone();
    f.place(Side::Right, 0, 0);
    assert!(save::begin(f.m(), &svc, vec![Side::Right], vec![SaveTarget::InPlace]).is_some());
    f.run(CommandId::DeleteItem);
    // The click on "D" as shown (line 3 before the queued Delete Line).
    input::press(f.m(), Side::Right, TextPos::new(3, 0), false, Clicks::One);
    f.run(CommandId::CopyRight);
    assert_eq!(input::queued(f.m()), 3);
    assert_eq!(
        f.text(Side::Right),
        right,
        "nothing applied during the save"
    );
    f.settle();
    assert_eq!(f.text(Side::Right), want);
    assert_eq!(std::fs::read(&f.paths[1]).expect("read"), right.as_bytes());
    // One undo step each.
    f.m().set_focus(Side::Right);
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Right), "b\nc\nD\n");
}

/// F3: header actions and undo of a frozen pane wait too; a key that is the view's (Ins) acts at once.
#[test]
fn a11_7_header_actions_and_undo_during_a_save_wait() {
    use crate::text::input::{self, Key, Press};
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 3);
    f.type_text("1");
    let svc = f.world.service.clone();
    assert!(save::begin(f.m(), &svc, vec![Side::Left], vec![SaveTarget::InPlace]).is_some());
    f.run(CommandId::Undo);
    assert!(!crate::text::header::set_line_endings(
        f.m(),
        Side::Left,
        hikasa_text::LineEnding::CrLf
    ));
    assert!(f.key(Press::plain(Key::Insert)), "Ins is the view's");
    assert!(f.m().display().overwrite);
    assert_eq!(input::queued(f.m()), 2);
    f.settle();
    assert_eq!(
        f.text(Side::Left),
        "abc\r\n",
        "the undo, then the conversion"
    );
}
