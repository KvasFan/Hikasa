//! A11.9 (D-302, D-303) and A11.10's GUI part (D-304): the File Info header's items for fixtures (UTF-16LE with
//! BOM, windows-1252, mixed endings, a read-only file, `--ro2`), a path Accept and *Reopen with Encoding* reopening
//! one side (the other keeps its edits), *Save with Encoding* and the line-ending conversions as steps, the format
//! picker reliving; the status bar's items per caret move, `Col 7 (13)` with tabs, the element under the caret,
//! *Recomparing…* after 250 ms; a format pick on a modified view keeps the undo log and the caret.

use hikasa_core::Side;
use hikasa_text::LineEnding;

use crate::commands::CommandId;
use crate::tests_text::Fixture;
use crate::text::header::{self, Badge};
use crate::text::live::{DEBOUNCE_S, SHOW_AFTER_S};

#[test]
fn a11_9_header_items_of_the_fixtures() {
    let world = crate::tests::World::new();
    let l = world.path("work/u16.txt");
    let mut bytes = vec![0xFF, 0xFE];
    for u in "a\r\nb\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&l, bytes).expect("write");
    let r = world.path("work/w1252.txt");
    std::fs::write(&r, b"caf\xe9\nmore\n").expect("write");
    let mut f = Fixture::open_paths(world, l, r);
    let left = header::items(f.m(), Side::Left).expect("items");
    assert_eq!(left.encoding, "UTF-16LE BOM");
    assert_eq!(left.eol, "MIX");
    assert!(left.utf && left.bom);
    assert!(left.path.ends_with("u16.txt"));
    let right = header::items(f.m(), Side::Right).expect("items");
    assert_eq!(right.encoding, "windows-1252");
    assert_eq!(right.eol, "LF");
    assert_eq!(right.format, "Everything Else");
    // --ro2: Editing disabled.
    f.m().set_write_policy(crate::folder_view::WritePolicy {
        read_only: hikasa_folder::ops::ReadOnlySides {
            left: false,
            right: true,
        },
        ..crate::folder_view::WritePolicy::default()
    });
    let right = header::items(f.m(), Side::Right).expect("items");
    assert!(matches!(
        right.badges.first(),
        Some(Badge::Blocked {
            text: "Editing disabled",
            ..
        })
    ));
}

#[cfg(unix)]
#[test]
fn a11_9_a_read_only_file_says_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let world = crate::tests::World::new();
    let l = world.write("work/a.c", "int a;\n");
    let r = world.write("work/b.c", "int b;\n");
    std::fs::set_permissions(&l, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    let mut f = Fixture::open_paths(world, l, r);
    let items = header::items(f.m(), Side::Left).expect("items");
    if items.badges.is_empty() {
        // Root may write anything: no block to show.
        return;
    }
    assert!(matches!(
        items.badges[0],
        Badge::Blocked {
            text: "Read-only",
            ..
        }
    ));
    assert_eq!(items.format, "Detected (C, C++, C#, ObjC Source)");
}

#[test]
fn a11_9_header_actions_are_steps_or_reopens() {
    let mut f = Fixture::new("abc\r\nd\n", "abc\n");
    // The line-ending menu: one step.
    assert!(header::set_line_endings(f.m(), Side::Left, LineEnding::Lf));
    assert_eq!(f.text(Side::Left), "abc\nd\n");
    assert_eq!(header::items(f.m(), Side::Left).expect("items").eol, "LF");
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), "abc\r\nd\n");
    // Save with Encoding: a SetEncoding step.
    assert!(header::save_with_encoding(f.m(), Side::Left, "UTF-16BE"));
    assert_eq!(
        header::items(f.m(), Side::Left).expect("items").encoding,
        "UTF-16BE"
    );
    f.run(CommandId::Undo);
    assert_eq!(
        header::items(f.m(), Side::Left).expect("items").encoding,
        "UTF-8"
    );
    // A path Accept reopens that side; the other keeps its edits.
    f.place(Side::Right, 0, 0);
    f.type_text("R");
    let other = f.world.write("work/other.c", "zzz\n");
    f.m()
        .request_reopen(Side::Left, Some(other.clone().into_os_string()), None);
    f.settle();
    assert_eq!(f.text(Side::Left), "zzz\n");
    assert_eq!(
        f.text(Side::Right),
        "Rabc\n",
        "the other side keeps its edits"
    );
    assert!(header::title(f.m(), Side::Left).ends_with("other.c"));
    // A modified side asks first.
    f.place(Side::Left, 0, 0);
    f.type_text("q");
    f.m()
        .request_reopen(Side::Left, None, Some(Some("windows-1252".into())));
    f.settle();
    assert!(f.prompt().is_some_and(|p| p.starts_with("Save changes to")));
    f.answer(crate::dialog::ButtonRole::No);
    assert_eq!(f.text(Side::Left), "zzz\n");
    assert_eq!(
        header::items(f.m(), Side::Left).expect("items").encoding,
        "windows-1252"
    );
}

#[test]
fn a11_10_a_format_pick_relives_and_keeps_the_edits() {
    let mut f = Fixture::new("int a = 1;\nint b;\n", "int a = 2;\nint b;\n");
    f.place(Side::Left, 1, 3);
    f.type_text("X");
    let caret = f.caret(Side::Left);
    let installs = f.m().installs();
    let name = f
        .m()
        .format_names()
        .iter()
        .find(|n| !n.starts_with("C,"))
        .cloned()
        .expect("another text format");
    f.m().request_format(Side::Left, Some(name.clone()));
    f.settle();
    assert!(f.m().installs() > installs);
    assert_eq!(
        header::items(f.m(), Side::Left).expect("items").format,
        format!("Forced ({name})")
    );
    assert_eq!(f.caret(Side::Left), caret, "the caret stays");
    assert!(f.app.enabled(CommandId::Undo), "the undo log stays");
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), "int a = 1;\nint b;\n");
}

#[test]
fn a11_9_status_items_per_caret_move() {
    let mut f = Fixture::new("\tint x = 1; // c\nint y;\n", "\tint x = 2; // c\nint y;\n");
    f.place(Side::Left, 0, 4);
    let items = f.m().status_items();
    assert!(
        items.right.contains(&"Ln 1, Col 5 (8)".to_owned()),
        "{:?}",
        items.right
    );
    assert!(items.right.contains(&"INS".to_owned()));
    assert!(
        items.right.contains(&"Important Difference".to_owned()),
        "{:?}",
        items.right
    );
    assert!(items.right.contains(&"UTF-8".to_owned()));
    assert!(items.right.contains(&"LF".to_owned()));
    // The element under the caret: the comment.
    f.place(Side::Left, 0, 14);
    let items = f.m().status_items();
    assert!(
        items.right.iter().any(|i| i.contains("Comment")),
        "{:?}",
        items.right
    );
    // Recomparing… after 250 ms of a running job (not polled meanwhile).
    f.type_text("z");
    let t = f.now + DEBOUNCE_S;
    f.m().set_now(t);
    f.app.handle(crate::app::UiMsg::Tick);
    if f.m().recompare_state().running.is_some() {
        f.m().set_now(t + SHOW_AFTER_S + 0.01);
        let items = f.m().status_items();
        assert!(
            items.right.contains(&"Recomparing…".to_owned()),
            "{:?}",
            items.right
        );
    }
    f.settle();
}
