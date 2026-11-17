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

// ---- §1.11.35 W11a fixes and the review's gaps ------------------------------------------------------------------

/// A user text format with a characters-per-line limit of 8 and a tab size of 3.
const NARROW: &str = "schema_version = 1\nid = \"narrow\"\nname = \"Narrow\"\ndescription = \"Eight characters a \
                      line.\"\nkind = \"text\"\nmask = \"*.nar\"\n\n[text]\ntab_size = 3\nchars_per_line_limit = 8\n";

/// F13: a pick back to *Detected* takes the editing options of the format the comparison chose.
#[test]
fn a11_9_a_pick_back_to_detected_takes_the_detected_formats_options() {
    let mut f = Fixture::new("int a;\n", "int b;\n");
    let before = f.doc(Side::Left).options().clone();
    f.m().request_format(Side::Left, Some("HTML".into()));
    f.settle();
    assert_eq!(f.doc(Side::Left).options().tab_size, 2, "HTML's tab size");
    f.m().request_format(Side::Left, None);
    f.settle();
    assert!(
        header::items(f.m(), Side::Left)
            .expect("items")
            .format
            .starts_with("Detected (C")
    );
    assert_eq!(*f.doc(Side::Left).options(), before, "C's options again");
}

/// G24, G25, F13: a pick of a format with another characters-per-line limit re-wraps the document and maps the caret;
/// back to *Detected* the limit goes again.
#[test]
fn a11_10_a_format_with_a_line_limit_rewraps_and_maps_the_caret() {
    let world = crate::tests::World::with_files(&[("formats/narrow.toml", NARROW)]);
    let l = world.write("work/a.c", "int alpha = 12345;\nint b;\n");
    let r = world.write("work/b.c", "int alpha = 12345;\nint c;\n");
    let mut f = Fixture::open_paths(world, l, r);
    f.place(Side::Left, 0, 10);
    f.m().request_format(Side::Left, Some("Narrow".into()));
    f.settle();
    let doc = f.doc(Side::Left);
    assert_eq!(doc.options().chars_per_line_limit, Some(8), "the new limit");
    assert!(doc.position_lines() > 3, "re-wrapped into pieces of 8");
    let caret = f.caret(Side::Left);
    assert_eq!(
        (caret.line.0, caret.byte),
        (1, 2),
        "byte 10 of the line: piece 1, byte 2"
    );
    f.m().request_format(Side::Left, None);
    f.settle();
    let doc = f.doc(Side::Left);
    assert_eq!(doc.options().chars_per_line_limit, None);
    assert_eq!(doc.position_lines(), 3);
    let caret = f.caret(Side::Left);
    assert_eq!((caret.line.0, caret.byte), (0, 10));
}

/// F14: a reopen cancelled at its prompt leaves nothing behind: the next reopen starts from the view's request.
#[test]
fn a11_9_a_cancelled_reopen_leaves_no_request() {
    let mut f = Fixture::new("abc\n", "abd\n");
    let other = f.world.write("work/other.c", "zzz\n");
    let original = f.m().request().left.clone();
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.m()
        .request_reopen(Side::Left, Some(other.into_os_string()), None);
    f.settle();
    assert!(f.prompt().is_some_and(|p| p.contains("Reopening")));
    f.answer(crate::dialog::ButtonRole::Cancel);
    f.m()
        .request_reopen(Side::Right, None, Some(Some("windows-1252".into())));
    f.settle();
    assert_eq!(f.text(Side::Left), "xabc\n");
    assert_eq!(f.m().request().left, original);
    assert_eq!(f.m().take_reopen_request(Side::Left).left, original);
}

/// F15: after a path Accept the tab's caption names the reopened file.
#[test]
fn a11_9_a_path_accept_renames_the_caption() {
    let mut f = Fixture::new("abc\n", "abd\n");
    let other = f.world.write("work/other.c", "zzz\n");
    f.m()
        .request_reopen(Side::Left, Some(other.into_os_string()), None);
    f.settle();
    assert_eq!(f.text(Side::Left), "zzz\n");
    let caption = f.m().caption();
    assert!(
        caption.contains("other.c") && !caption.contains("left.c"),
        "{caption}"
    );
}

/// G30: a Save As renames the pane in the caption.
#[test]
fn a11_7_save_as_renames_the_caption() {
    let mut f = Fixture::new("abc\n", "abd\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::SaveAs);
    let target = f.world.path("work/renamed.c");
    let view = f.view;
    f.browse_answer(
        crate::browse::BrowseTarget::TextSaveAs {
            view,
            side: Side::Left,
            local: false,
        },
        Some(&target),
    );
    let caption = f.m().caption();
    assert!(
        caption.contains("renamed.c") && !caption.contains("left.c"),
        "{caption}"
    );
}

/// G21: the header names CRLF and CR files by their terminators.
#[test]
fn a11_9_crlf_and_cr_files_in_the_header() {
    let mut f = Fixture::new("a\r\nb\r\n", "a\rb\r");
    assert_eq!(header::items(f.m(), Side::Left).expect("items").eol, "CRLF");
    assert_eq!(header::items(f.m(), Side::Right).expect("items").eol, "CR");
}

/// A11.9: a zip member's header (its path inside the archive), `clipboard://` (no place), a Sorted format
/// (*Converted*, not editable).
#[test]
fn a11_9_header_of_a_zip_member_the_clipboard_and_a_sorted_format() {
    let world = crate::tests::World::new();
    let zip = crate::tests_shell::stored_zip(&[("m.c", b"int m;\n")]);
    let z = world.path("work/z.zip");
    std::fs::write(&z, zip).expect("zip");
    let r = world.write("work/r.c", "int r;\n");
    let mut f = Fixture::open_paths(world, z.join("m.c"), r);
    let items = header::items(f.m(), Side::Left).expect("items");
    assert!(
        items.path.contains("z.zip") && items.path.ends_with("m.c"),
        "{}",
        items.path
    );
    assert_eq!(f.text(Side::Left), "int m;\n");

    let mut f = crate::tests_text_save::clipboard_fixture("abc\n");
    let items = header::items(f.m(), Side::Left).expect("items");
    assert!(items.path.starts_with("clipboard"), "{}", items.path);
    assert!(f.doc(Side::Left).location().is_none());
    assert!(
        crate::text::save::save_target_of(f.m(), Side::Left).is_none(),
        "Save goes to Save As"
    );

    // Sorted: a conversion without a way back.
    let world = crate::tests::World::new();
    let l = world.write("work/s1.txt", "b\na\n");
    let r = world.write("work/s2.txt", "a\nb\n");
    let req = hikasa_session::api::TextCompareRequest {
        left: l.into_os_string(),
        right: r.into_os_string(),
        format: Some("Sorted".into()),
        ..hikasa_session::api::TextCompareRequest::default()
    };
    let open = hikasa_session::api::GuiOpenRequest::for_text(
        &req,
        &hikasa_session::api::ViewWords::default(),
        false,
    );
    let resolved = world.service.resolve_open(&open).expect("resolve");
    let view = world
        .service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .expect("view request");
    let mut app = world.app(None);
    let target = app.open_target();
    let id = app.open_local(
        resolved.kind,
        view,
        &resolved.options,
        resolved.specs,
        target,
    );
    world.settle(&mut app);
    let m = crate::tests::text_model(&mut app, id);
    let items = header::items(m, Side::Left).expect("items");
    assert!(
        items.badges.contains(&Badge::Converted),
        "{:?}",
        items.badges
    );
    assert!(!m.doc(Side::Left).expect("doc").editability().editable());
}

/// G28: the status bar's element under a caret at a token's end (after the last character of a comment).
#[test]
fn a11_9_the_element_at_a_tokens_end() {
    let mut f = Fixture::new("int x; // note\n", "int y; // note\n");
    f.settle();
    f.place(Side::Left, 0, 14);
    let items = f.m().status_items();
    assert!(
        items.right.iter().any(|i| i.contains("Comment")),
        "{:?}",
        items.right
    );
}

/// F14: a reopen waiting at its prompt and a reopen of the other side meanwhile keep their own changes: the other
/// side's runs with the left's file unchanged, and Discard then reopens the left with its new file.
#[test]
fn a11_9_two_reopens_keep_their_own_changes() {
    let mut f = Fixture::new("abc\n", "abd\n");
    let other = f.world.write("work/other.c", "zzz\n");
    let original = f.m().request().left.clone();
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.m()
        .request_reopen(Side::Left, Some(other.clone().into_os_string()), None);
    f.settle();
    assert!(f.prompt().is_some_and(|p| p.contains("Reopening")));
    // The right side (unmodified) reopens at once with another encoding.
    f.m()
        .request_reopen(Side::Right, None, Some(Some("windows-1252".into())));
    f.settle();
    assert_eq!(
        f.m().request().left,
        original,
        "the left's waiting change is not taken"
    );
    assert_eq!(f.text(Side::Left), "xabc\n");
    f.answer(crate::dialog::ButtonRole::No);
    assert_eq!(
        f.text(Side::Left),
        "zzz\n",
        "Discard reopens the left with its file"
    );
    assert_eq!(f.m().request().left, other.into_os_string());
    assert_eq!(
        header::items(f.m(), Side::Right).expect("items").encoding,
        "windows-1252"
    );
}
