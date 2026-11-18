//! M2 W13c acceptance A13.21 (D-369), the W13c rows: *Compare Files Using* from each of the four file views opens the
//! view's specs in a new tab of the chosen kind (the parent link kept, unsaved text edits not carried, with a message
//! panel line); the text view's binary notice *Compare in Hex*.

use std::ffi::OsString;

use hikasa_core::Side;
use hikasa_text::{EditRequest, TextPos};

use crate::commands::{CommandId, Scope};
use crate::tests::{TestApp, World, text_model};
use crate::window::{CommandArg, Invocation, OpenWithItem, ViewModel};

/// The specs of the active tab's file view.
fn specs(app: &TestApp) -> Option<[OsString; 2]> {
    let tab = app.active_tab()?;
    match tab.model()? {
        ViewModel::Text(m) => Some([m.request().left.clone(), m.request().right.clone()]),
        ViewModel::Table(m) => Some([m.request().left.clone(), m.request().right.clone()]),
        ViewModel::Hex(m) => Some([m.request().left.clone(), m.request().right.clone()]),
        ViewModel::Picture(m) => Some([m.request().left.clone(), m.request().right.clone()]),
        _ => None,
    }
}

fn using(item: OpenWithItem) -> Invocation {
    Invocation::with(CommandId::CompareUsingViewList, CommandArg::OpenWith(item))
}

/// From each of the four views to each other kind: a new tab of that kind with the same specs.
#[test]
fn a13_21_compare_files_using_from_each_view() {
    let kinds = [
        (Scope::Text, OpenWithItem::TextCompare),
        (Scope::Table, OpenWithItem::TableCompare),
        (Scope::Hex, OpenWithItem::HexCompare),
        (Scope::Picture, OpenWithItem::PictureCompare),
    ];
    for (from, from_item) in kinds {
        for (to, item) in kinds {
            if to == from {
                continue;
            }
            let world = crate::tests_hex_open::hex_world();
            let mut app = world.app(None);
            let pair = crate::tests_picture_open::pair(&world);
            app.0.open_specs_for_test(&world.service, &pair, from_item);
            world.settle(&mut app);
            assert_eq!(app.scope(), from, "{from:?}");
            let before = specs(&app).expect("specs");
            let tabs = app.tabs().len();
            assert!(app.enabled(CommandId::CompareUsingViewList));
            app.execute(using(item));
            world.settle(&mut app);
            assert_eq!(app.tabs().len(), tabs + 1, "{from:?} → {to:?}");
            assert_eq!(app.scope(), to, "{from:?} → {to:?}");
            assert_eq!(specs(&app).expect("specs"), before, "{from:?} → {to:?}");
        }
    }
}

/// Unsaved edits are not carried: the new view compares the files, a message panel line says so; the parent link of
/// a child view is kept.
#[test]
fn a13_21_unsaved_edits_and_the_parent_link() {
    let world = World::new();
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let at = TextPos::new(0, 0);
    assert!(crate::text::edit::edit(
        text_model(&mut app, view),
        Side::Left,
        EditRequest::Replace {
            range: at..at,
            text: "X".into(),
            keep_eols: false,
        },
        crate::text::edit::Origin::Typing { at_ms: 0 },
        None,
    ));
    app.execute(using(OpenWithItem::HexCompare));
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex);
    let message = app
        .tab(view)
        .and_then(|t| t.message.as_ref())
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(message.contains("unsaved edits"), "{message}");
    // A child of a folder view keeps its parent link.
    crate::tests_text_link::parent(&world);
    let (_, child) = crate::tests_text_link::child_of(&world, &mut app, "f1.txt");
    assert_eq!(app.scope(), Scope::Text);
    let parent = |app: &TestApp, view| {
        app.tab(view)
            .and_then(|t| t.session.as_ref())
            .and_then(|s| s.parent.clone())
            .filter(|p| !p.is_empty())
    };
    let link = parent(&app, child).expect("a child view");
    app.execute(using(OpenWithItem::HexCompare));
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex);
    let opened = app.active_tab().expect("tab").id;
    assert_ne!(opened, child);
    assert_eq!(
        parent(&app, opened),
        Some(link),
        "the parent link carries over"
    );
}

/// The binary notice's *Compare in Hex* opens the pair in Hex Compare.
#[test]
fn a13_21_compare_in_hex_of_the_binary_notice() {
    let world = World::new();
    let mut app = world.app(None);
    let l = world.path("work/a.dat");
    let r = world.path("work/b.dat");
    std::fs::write(&l, b"\x00\x01\x02binary\x00").expect("write");
    std::fs::write(&r, b"\x00\x01\x03binary\x00").expect("write");
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    assert_eq!(
        app.scope(),
        Scope::Text,
        "no Hex format: Text Compare (TC-003)"
    );
    text_model(&mut app, view).compare_in_hex();
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Hex);
    assert_eq!(
        specs(&app).expect("specs"),
        [l.into_os_string(), r.into_os_string()]
    );
}

/// The comparison's FF-002 format overrides are carried to the new view (W13c review: mutation G11).
#[test]
fn a13_21_compare_files_using_carries_the_format_overrides() {
    let world = crate::tests_hex_open::hex_world();
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    let request = hikasa_session::api::GuiOpenRequest {
        specs: vec![l.into_os_string(), r.into_os_string()],
        ..hikasa_session::api::GuiOpenRequest::default()
    };
    let resolved = world.service.resolve_open(&request).expect("resolve");
    let mut view = world
        .service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .expect("view request");
    let overrides = std::collections::BTreeMap::from([("c-cpp".to_owned(), false)]);
    match &mut view {
        hikasa_session::api::ViewRequest::Text { request, .. } => {
            request.format_overrides = overrides.clone();
        }
        _ => panic!("a text pair"),
    }
    let target = app.open_target();
    app.open_local(
        resolved.kind,
        view,
        &resolved.options,
        resolved.specs,
        target,
    );
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    for (to, item) in [
        (Scope::Hex, OpenWithItem::HexCompare),
        (Scope::Table, OpenWithItem::TableCompare),
    ] {
        let text = app
            .tabs()
            .iter()
            .position(|t| matches!(t.model(), Some(ViewModel::Text(_))))
            .expect("the text view");
        app.activate(text);
        app.execute(using(item));
        world.settle(&mut app);
        assert_eq!(app.scope(), to);
        let carried = match app.active_tab().and_then(|t| t.model()) {
            Some(ViewModel::Hex(m)) => m.request().format_overrides.clone(),
            Some(ViewModel::Table(m)) => m.request().format_overrides.clone(),
            _ => panic!("{to:?}"),
        };
        assert_eq!(carried, overrides, "{to:?}");
    }
}
