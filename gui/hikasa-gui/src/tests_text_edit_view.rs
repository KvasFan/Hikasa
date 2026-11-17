//! A11.20 (D-313): the Text Edit view. Every way of §1.11.22 opens it (Edit Text File, Open With in a folder pane and a
//! text header, Home ▸ New, a `text-edit` session, `--edit` with one and two specs; a Find in Files hit is A11.15's);
//! `Scope::TextEdit` has no two-pane entry; save, Save As, D-301, find, conversions and bookmarks work in it;
//! `ViewRequest::TextEdit` round trips through `session_for_view`, `TabDuplicate` and a workspace;
//! `ViewOutcome::for_text_edit`.

use hikasa_core::Side;
use hikasa_session::api::{TextEditRequest, ViewOutcome, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use hikasa_session::sessions::SessionType;
use hikasa_text::edit::{TextPos, doc_end};

use crate::commands::{CommandId, Scope, lookup};
use crate::dialog::ButtonRole;
use crate::tests::{TestApp, World, text_model};
use crate::tests_text::Fixture;
use crate::window::{CommandArg, Invocation, OpenWithItem, ViewModel};

/// Opens `path` in a Text Edit view of `app` and settles.
fn open_edit(world: &World, app: &mut TestApp, path: &std::path::Path) -> ViewId {
    let request = ViewRequest::TextEdit {
        request: TextEditRequest {
            spec: path.as_os_str().to_owned(),
            ..TextEditRequest::default()
        },
    };
    let target = app.open_target();
    let view = app.0.open_local(
        ViewKind::TextEdit,
        request,
        &OpenOptions::default(),
        vec![path.display().to_string()],
        target,
    );
    world.settle(app);
    view
}

/// The Text Edit views of the window, with their files.
fn edit_views(app: &mut TestApp) -> Vec<(ViewId, String)> {
    app.tabs()
        .iter()
        .filter_map(|t| match t.model() {
            Some(ViewModel::Text(m)) if m.is_edit() => Some((t.id, m.inputs()[0].clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a11_20_every_way_opens_it() {
    let world = World::new();
    let a = world.write("work/a.txt", "alpha\n");
    let b = world.write("work/b.txt", "beta\n");
    let mut app = world.app(None);
    // Tools ▸ Edit Text File: the browse dialog in file mode, then a view.
    app.execute(CommandId::EditTextFile);
    let win = app.win();
    let step = crate::dialog::DialogStep::CloseWith(vec![crate::dialog::Followup::Browsed {
        target: crate::browse::BrowseTarget::EditTextFile,
        answer: Some((a.display().to_string(), false)),
        show_hidden: false,
    }]);
    app.0.dialog_step_pub(win, step);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::TextEdit);
    assert_eq!(edit_views(&mut app).len(), 1);
    // Home ▸ New ▸ Text Edit: a path and Compare.
    app.0.execute(
        win,
        Invocation::with(
            CommandId::SessionNewList,
            CommandArg::SessionType(SessionType::TextEdit),
        ),
    );
    world.settle(&mut app);
    if let Some(ViewModel::Home(h)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        h.panel.left = b.display().to_string();
        assert!(h.ready(), "one file is enough");
    }
    app.execute(CommandId::HomeCompare);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::TextEdit);
    assert_eq!(edit_views(&mut app).len(), 2);
    // Open With ▸ Text Edit of a text header: the focus pane's file.
    let c = world.write("work/c.txt", "c\n");
    let d = world.write("work/d.txt", "d\n");
    let compare = world.open(&mut app, &c, &d);
    world.settle(&mut app);
    text_model(&mut app, compare).set_focus(Side::Right);
    app.0.execute(
        win,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::TextEdit),
        ),
    );
    world.settle(&mut app);
    assert!(
        edit_views(&mut app)
            .iter()
            .any(|(_, s)| s.ends_with("d.txt"))
    );
    // A `text-edit` session: `session_view_of` gives the view's request.
    let (session, _) = world
        .service
        .text_edit_session(
            &TextEditRequest {
                spec: c.as_os_str().to_owned(),
                ..TextEditRequest::default()
            },
            Default::default(),
            Default::default(),
            None,
        )
        .expect("session");
    let opened = world
        .service
        .session_view_of(&session, "edit".into())
        .expect("a view");
    assert_eq!(opened.kind, ViewKind::TextEdit);
    assert!(
        matches!(&opened.request, ViewRequest::TextEdit { request } if request.spec == c.as_os_str())
    );
    // `--edit a b`: one view per spec (D-278), each its own request.
    let request = hikasa_session::api::GuiOpenRequest {
        specs: vec![a.as_os_str().to_owned(), b.as_os_str().to_owned()],
        kind: Some(ViewKind::TextEdit),
        ..Default::default()
    };
    let resolved = world.service.resolve_open(&request).expect("resolved");
    assert_eq!(resolved.requests(false).len(), 2);
    for spec in &resolved.specs {
        let view = world
            .service
            .open_view_request(
                ViewKind::TextEdit,
                std::slice::from_ref(spec),
                &resolved.options,
            )
            .expect("a request");
        assert!(matches!(view, ViewRequest::TextEdit { .. }));
    }
    let _ = a;
}

#[test]
fn a11_20_open_with_text_edit_in_a_folder_pane() {
    let world = World::new();
    let l = world.write("fl/x.txt", "x\n");
    world.write("fr/x.txt", "a longer one\n");
    let mut app = world.app(None);
    let view = world.open(&mut app, &world.path("fl"), &world.path("fr"));
    world.settle(&mut app);
    crate::tests_folder_ops::select(&mut app.0, view, "x.txt");
    let win = app.win();
    app.0.execute(
        win,
        Invocation::with(
            CommandId::OpenWithList,
            CommandArg::OpenWith(OpenWithItem::TextEdit),
        ),
    );
    world.settle(&mut app);
    let views = edit_views(&mut app);
    assert!(
        views.iter().any(|(_, s)| *s == l.display().to_string()),
        "{views:?}"
    );
}

#[test]
fn a11_20_its_rows_have_no_two_pane_entry() {
    for id in [
        CommandId::CopyRight,
        CommandId::CopyLeft,
        CommandId::SideBySide,
        CommandId::ZoomPane,
        CommandId::ShowAlignDetails,
        CommandId::DiffSectionNext,
        CommandId::AlignWith,
        CommandId::Ignored,
        CommandId::GoLeft,
        CommandId::DiffFileNext,
        CommandId::Rules,
    ] {
        assert!(lookup(Scope::TextEdit, id).is_none(), "{id:?}");
    }
    for id in [
        CommandId::Save,
        CommandId::SaveAs,
        CommandId::Find,
        CommandId::Replace,
        CommandId::FindInFiles,
        CommandId::GoTo,
        CommandId::BookmarkToggle1,
        CommandId::EditNext,
        CommandId::TrimTrailWS,
        CommandId::ShowTextDetails,
        CommandId::ShowHexDetails,
        CommandId::Info,
        CommandId::SessionClear,
        CommandId::CompareToClipboard,
    ] {
        assert!(lookup(Scope::TextEdit, id).is_some(), "{id:?}");
    }
    assert_eq!(Scope::TextEdit.help_topic(), "view-text-edit");
    assert_eq!(Scope::TextEdit.type_title(), "Text Edit");
}

#[test]
fn a11_20_save_find_convert_and_bookmarks() {
    let world = World::new();
    let path = world.write("work/e.txt", "one  \ntwo\n");
    let mut app = world.app(None);
    let view = open_edit(&world, &mut app, &path);
    let text = |app: &mut TestApp| {
        let d = text_model(app, view).doc(Side::Left).expect("doc").clone();
        d.text(TextPos::ZERO..doc_end(&d))
    };
    // A conversion, a bookmark and a find in the one pane.
    {
        let m = text_model(&mut app, view);
        m.pane_mut(Side::Left).set_caret(TextPos::new(1, 0), false);
        m.caret_moved();
    }
    app.execute(CommandId::TrimTrailWS);
    assert_eq!(text(&mut app), "one\ntwo\n");
    {
        let m = text_model(&mut app, view);
        m.pane_mut(Side::Left).set_caret(TextPos::new(1, 0), false);
        m.caret_moved();
    }
    app.execute(CommandId::BookmarkToggle1);
    {
        let m = text_model(&mut app, view);
        m.pane_mut(Side::Left).set_caret(TextPos::ZERO, false);
    }
    app.execute(CommandId::BookmarkGoTo1);
    assert_eq!(
        text_model(&mut app, view).pane(Side::Left).caret,
        TextPos::new(1, 0)
    );
    app.execute(CommandId::Find);
    {
        let m = text_model(&mut app, view);
        m.find.query.text = "two".into();
        m.find.ask = Some(crate::text::find::FindAction::Next);
    }
    world.settle(&mut app);
    let sel = text_model(&mut app, view)
        .pane(Side::Left)
        .selection()
        .range();
    assert_eq!(sel, TextPos::new(1, 0)..TextPos::new(1, 3));
    // Save writes the one file (side `None`).
    app.execute(CommandId::Save);
    world.settle(&mut app);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "one\ntwo\n");
    assert!(
        !text_model(&mut app, view)
            .doc(Side::Left)
            .expect("doc")
            .is_modified()
    );
    // Save As moves the file.
    let other = world.path("work/f.txt");
    app.execute(CommandId::SaveAs);
    let win = app.win();
    let step = crate::dialog::DialogStep::CloseWith(vec![crate::dialog::Followup::Browsed {
        target: crate::browse::BrowseTarget::TextSaveAs {
            view,
            side: Side::Left,
            local: false,
        },
        answer: Some((other.display().to_string(), false)),
        show_hidden: false,
    }]);
    app.0.dialog_step_pub(win, step);
    world.settle(&mut app);
    assert_eq!(std::fs::read_to_string(&other).expect("read"), "one\ntwo\n");
    assert!(
        app.tab(view)
            .is_some_and(|t| t.caption().starts_with("f.txt"))
    );
}

#[test]
fn a11_20_close_with_edits_asks() {
    let world = World::new();
    let path = world.write("work/e.txt", "abc\n");
    let mut f = Fixture::new("x\n", "y\n");
    let _ = &world;
    let view = open_edit(&f.world, &mut f.app, &path);
    f.view = view;
    f.place(Side::Left, 0, 0);
    f.type_text("Z");
    assert_eq!(f.m().caption(), "e.txt*");
    f.run(CommandId::CloseTab);
    assert!(f.prompt().is_some_and(|p| p.contains("e.txt")));
    f.answer(ButtonRole::Cancel);
    assert!(f.app.tab(view).is_some());
}

#[test]
fn a11_20_its_request_round_trips() {
    let world = World::new();
    let path = world.write("work/e.txt", "abc\n");
    let mut app = world.app(None);
    let view = open_edit(&world, &mut app, &path);
    // session_for_view → session_view_of.
    let (request, _) = text_model(&mut app, view).duplicate();
    let (session, _) = world
        .service
        .session_for_view(&request, Default::default(), Default::default(), None)
        .expect("session");
    assert_eq!(session.kind, SessionType::TextEdit);
    assert_eq!(session.specs.left, path.display().to_string());
    let back = world
        .service
        .session_view_of(&session, String::new())
        .expect("view");
    assert!(
        matches!(back.request, ViewRequest::TextEdit { ref request } if request.spec == path.as_os_str())
    );
    // TabDuplicate: a second Text Edit view of the file.
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    assert_eq!(edit_views(&mut app).len(), 2);
    // A workspace keeps it (its session inline, type `text-edit`).
    crate::tests_sessions::save_workspace(&world, &mut app, "edits");
    let name = hikasa_session::sessions::SessionName::parse("edits").expect("a name");
    let dto = world.service.workspace_show(&name).expect("show");
    let json = serde_json::to_string(&dto.workspace).expect("json");
    assert!(json.contains("text-edit"), "{json}");
}

#[test]
fn a11_20_the_outcome() {
    assert_eq!(ViewOutcome::for_text_edit(true), ViewOutcome::NoVerdict);
    assert_eq!(ViewOutcome::for_text_edit(false), ViewOutcome::LoadFailed);
    let world = World::new();
    let mut app = world.app(None);
    let ok = world.write("work/e.txt", "abc\n");
    let view = open_edit(&world, &mut app, &ok);
    let service = world.service.clone();
    assert_eq!(
        text_model(&mut app, view).outcome(&service),
        ViewOutcome::NoVerdict
    );
    let missing = world.path("work/missing.txt");
    let view = open_edit(&world, &mut app, &missing);
    assert_eq!(
        text_model(&mut app, view).outcome(&service),
        ViewOutcome::LoadFailed
    );
}
