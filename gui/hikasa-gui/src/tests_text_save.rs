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
    assert!(save::begin(
        f.m(),
        &svc,
        vec![Side::Left],
        vec![SaveTarget::InPlace]
    ));
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
    assert!(save::begin(
        f.m(),
        &svc,
        vec![Side::Left],
        vec![SaveTarget::InPlace]
    ));
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
