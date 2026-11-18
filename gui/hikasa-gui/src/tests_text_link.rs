//! A11.18 (D-311): the session link of a child text view. Next and Previous Difference Files with the parent folder view
//! open and closed (the parent compared again in a job), the end's message, D-301's question for edits; Copy File and
//! Open Next: the confirmation's steps are core's `text_copy_file_steps`, a declined copy stops the chain, a read-only
//! target greys the command.

use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{Session, SessionRef, SessionType};

use crate::app::OpenHow;
use crate::app::OpenTarget;
use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::tests::{TestApp, World, text_model};
use crate::tests_text::Fixture;

/// Three differing pairs and a same one, in a saved folder session `tree`.
pub(crate) fn parent(world: &World) {
    for (name, l, r) in [
        ("f1.txt", "one\n", "ONE!\n"),
        ("f2.txt", "two\n", "TWO!\n"),
        ("f3.txt", "three\n", "THREE!\n"),
        ("same.txt", "equal\n", "equal\n"),
    ] {
        world.write(&format!("pl/{name}"), l);
        world.write(&format!("pr/{name}"), r);
    }
    let mut s = Session::builtin(SessionType::FolderCompare);
    s.specs.left = world.path("pl").display().to_string();
    s.specs.right = world.path("pr").display().to_string();
    world
        .service
        .session_save(&SessionRef::parse("tree").expect("ref"), &s, false)
        .expect("save");
}

/// Opens the parent session and its child of `name`; returns (parent view, child view).
pub(crate) fn child_of(world: &World, app: &mut TestApp, name: &str) -> (ViewId, ViewId) {
    let target = OpenTarget::Window(app.win());
    app.0.open_named(
        SessionRef::parse("tree").expect("ref"),
        OpenHow::Target(target),
    );
    world.settle(app);
    let parent = app.active_tab().expect("tab").id;
    crate::tests_folder_ops::select(&mut app.0, parent, name);
    app.execute(CommandId::Open);
    world.settle(app);
    let child = app.active_tab().expect("tab").id;
    assert_ne!(child, parent);
    (parent, child)
}

/// The left file of a text view.
fn left_name(app: &mut TestApp, view: ViewId) -> String {
    let spec = text_model(app, view).inputs()[0].clone();
    crate::shared::spec_name(&spec).to_owned()
}

/// The view's message panel line.
fn message(app: &TestApp, view: ViewId) -> Option<String> {
    app.tab(view)
        .and_then(|t| t.message.as_ref())
        .map(|m| m.text.clone())
}

#[test]
fn a11_18_next_and_previous_with_the_parent_open() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    // An override of this pair does not carry over to the next one.
    text_model(&mut app, child).set_focus(Side::Left);
    app.execute(CommandId::Isolate);
    world.settle(&mut app);
    assert!(!crate::text::marks::anchors(text_model(&mut app, child)).is_empty());
    assert!(app.enabled(CommandId::DiffFileNext));
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    assert_eq!(left_name(&mut app, child), "f2.txt", "this view reopened");
    assert!(
        crate::text::marks::anchors(text_model(&mut app, child)).is_empty(),
        "the pair's overrides stay with it"
    );
    let link = app
        .view_session(child)
        .and_then(|s| s.parent.clone())
        .expect("link");
    assert_eq!(link.left, "f2.txt", "the link moved");
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    assert_eq!(left_name(&mut app, child), "f3.txt");
    // The end: the message panel (the same pair is no difference file).
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    assert_eq!(left_name(&mut app, child), "f3.txt");
    assert_eq!(
        message(&app, child).as_deref(),
        Some(crate::text::link::NO_MORE_FILES)
    );
    app.execute(CommandId::DiffFilePrev);
    world.settle(&mut app);
    assert_eq!(left_name(&mut app, child), "f2.txt");
}

#[test]
fn a11_18_the_parent_compared_again_when_closed() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (parent_view, child) = child_of(&world, &mut app, "f1.txt");
    let w = app.win();
    let index = app
        .window(w)
        .and_then(|x| x.index_of(parent_view))
        .expect("open");
    app.0
        .close_tab(w, index, hikasa_session::ipc::CloseReason::User);
    world.settle(&mut app);
    assert!(app.tab(parent_view).is_none());
    let index = app.window(w).and_then(|x| x.index_of(child)).expect("open");
    app.activate(index);
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    assert_eq!(left_name(&mut app, child), "f2.txt");
}

#[test]
fn a11_18_edits_ask_first() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    {
        let m = text_model(&mut app, child);
        m.set_focus(Side::Left);
        crate::text::input::text(m, "edit ");
    }
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let asked = {
        let w = app.win();
        app.window_mut(w)
            .and_then(|x| x.dialogs.top_mut())
            .and_then(|d| {
                d.as_any()
                    .downcast_ref::<crate::dialog::Prompt>()
                    .map(|p| p.message.clone())
            })
    };
    assert!(asked.is_some_and(|p| p.starts_with("Save changes to")));
    let w = app.win();
    let service = world.service.clone();
    let step = {
        let x = app.window_mut(w).expect("window");
        let p = x
            .dialogs
            .top_mut()
            .and_then(|d| d.as_any_mut().downcast_mut::<crate::dialog::Prompt>())
            .expect("prompt");
        crate::dialog::DialogModel::on_button(p, ButtonRole::No, &service)
    };
    app.0.dialog_step_pub(w, step);
    world.settle(&mut app);
    assert_eq!(
        left_name(&mut app, child),
        "f2.txt",
        "Discard: the next pair"
    );
}

#[test]
fn a11_18_copy_file_and_open_next() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    assert!(app.enabled(CommandId::CopyFileRight));
    app.execute(CommandId::CopyFileRight);
    world.settle(&mut app);
    let w = app.win();
    let prompt = app
        .window_mut(w)
        .and_then(|x| x.dialogs.top_mut())
        .and_then(|d| {
            d.as_any()
                .downcast_ref::<crate::dialog::Prompt>()
                .map(|p| p.message.clone())
        })
        .expect("the confirmation");
    // The steps as core plans them.
    assert!(prompt.contains("Copy f1.txt to the right"), "{prompt}");
    assert!(
        prompt.contains("Open the next pair: f2.txt ⇄ f2.txt"),
        "{prompt}"
    );
    assert!(!prompt.contains("Save the edits"), "no edits: {prompt}");
    assert!(!prompt.contains("Discard the edits"), "no edits: {prompt}");
    // Declined: nothing is copied, the view stays.
    let service = world.service.clone();
    let press = |app: &mut TestApp, role| {
        let step = {
            let x = app.window_mut(w).expect("window");
            let p = x
                .dialogs
                .top_mut()
                .and_then(|d| d.as_any_mut().downcast_mut::<crate::dialog::Prompt>())
                .expect("prompt");
            crate::dialog::DialogModel::on_button(p, role, &service)
        };
        app.0.dialog_step_pub(w, step);
        world.settle(app);
    };
    press(&mut app, ButtonRole::No);
    assert_eq!(
        std::fs::read_to_string(world.path("pr/f1.txt")).expect("read"),
        "ONE!\n"
    );
    assert_eq!(left_name(&mut app, child), "f1.txt");
    // Yes: copied, then the next pair.
    app.execute(CommandId::CopyFileRight);
    world.settle(&mut app);
    press(&mut app, ButtonRole::Yes);
    assert_eq!(
        std::fs::read_to_string(world.path("pr/f1.txt")).expect("read"),
        "one\n"
    );
    assert_eq!(left_name(&mut app, child), "f2.txt");
}

#[test]
fn a11_18_without_a_parent_or_toward_a_read_only_side() {
    let mut f = Fixture::new("a\n", "b\n");
    for id in [
        CommandId::DiffFileNext,
        CommandId::DiffFilePrev,
        CommandId::CopyFileRight,
        CommandId::CopyFileLeft,
    ] {
        assert!(!f.app.enabled(id), "{id:?} without a parent link");
    }
    let view = f.view;
    if let Some(s) = f.app.tab_mut(view).and_then(|t| t.session.as_mut()) {
        s.parent = Some(hikasa_session::sessions::ParentLink {
            session: "tree".into(),
            left: "a".into(),
            right: "b".into(),
        });
    }
    assert!(f.app.enabled(CommandId::CopyFileRight));
    let policy = crate::folder_view::WritePolicy {
        read_only: hikasa_folder::ops::ReadOnlySides {
            left: false,
            right: true,
        },
        ..crate::folder_view::WritePolicy::default()
    };
    f.m().set_write_policy(policy);
    assert!(
        !f.app.enabled(CommandId::CopyFileRight),
        "toward a read-only side"
    );
    assert!(f.app.enabled(CommandId::CopyFileLeft));
}

#[test]
fn a11_18_copy_file_lists_the_targets_discard() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    {
        let m = text_model(&mut app, child);
        m.set_focus(Side::Right);
        crate::text::input::text(m, "mine ");
    }
    app.execute(CommandId::CopyFileRight);
    world.settle(&mut app);
    let w = app.win();
    let prompt = app
        .window_mut(w)
        .and_then(|x| x.dialogs.top_mut())
        .and_then(|d| {
            d.as_any()
                .downcast_ref::<crate::dialog::Prompt>()
                .map(|p| p.message.clone())
        })
        .expect("the confirmation");
    assert!(
        prompt.contains("Discard the edits of f1.txt"),
        "the target's edits: {prompt}"
    );
    assert!(!prompt.contains("Save the edits"), "{prompt}");
}

// ---- W11b review fixes ------------------------------------------------------------------------------------------

/// Finding 7: Next Difference File builds the next pair's view as every text view is built (the format names, the
/// find panel's state) and keeps the view's display.
#[test]
fn w11b_review_next_difference_file_keeps_the_display() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    app.execute(CommandId::OverUnder);
    app.execute(CommandId::ShowRuler);
    text_model(&mut app, child).find.query.text = "kept".into();
    world.settle(&mut app);
    let before = {
        let m = text_model(&mut app, child);
        (
            m.display().clone(),
            m.format_names().to_vec(),
            m.find.query.text.clone(),
            *m.tweaks(),
        )
    };
    assert!(before.0.over_under && before.0.ruler && !before.1.is_empty());
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let m = text_model(&mut app, child);
    assert!(m.inputs()[0].ends_with("f2.txt"));
    let after = (
        m.display().clone(),
        m.format_names().to_vec(),
        m.find.query.text.clone(),
        *m.tweaks(),
    );
    assert_eq!(after, before);
}

/// Finding 7: the next pair's files are decoded as detected: an encoding or a format picked for the previous pair's
/// files does not carry over.
#[test]
fn w11b_review_next_difference_file_drops_the_picks_of_the_files() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    text_model(&mut app, child).request_reopen(Side::Left, None, Some(Some("windows-1252".into())));
    world.settle(&mut app);
    text_model(&mut app, child).request_format(Side::Right, Some("cobol".into()));
    world.settle(&mut app);
    assert!(
        text_model(&mut app, child)
            .request()
            .encodings
            .left
            .is_some()
    );
    assert!(
        text_model(&mut app, child)
            .request()
            .formats
            .right
            .is_some()
    );
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let m = text_model(&mut app, child);
    assert!(m.inputs()[0].ends_with("f2.txt"));
    assert_eq!(m.request().encodings, Default::default());
    assert_eq!(m.request().formats, Default::default());
}

/// B20: the pair's alignment overrides do not carry over to the next pair, also after a relive.
#[test]
fn w11b_review_next_pair_has_no_overrides() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    {
        let m = text_model(&mut app, child);
        m.set_focus(Side::Left);
    }
    app.execute(CommandId::Isolate);
    world.settle(&mut app);
    assert_eq!(
        crate::text::marks::anchors(text_model(&mut app, child)).len(),
        1
    );
    text_model(&mut app, child).request_format(Side::Left, Some("cobol".into()));
    world.settle(&mut app);
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let m = text_model(&mut app, child);
    assert!(m.inputs()[0].ends_with("f2.txt"));
    assert!(crate::text::marks::anchors(m).is_empty());
}

/// C35: Copy File and Open Next names the source and the target by their sides.
#[test]
fn w11b_review_copy_steps_name_their_sides() {
    let world = World::new();
    parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (_, child) = child_of(&world, &mut app, "f1.txt");
    {
        let m = text_model(&mut app, child);
        m.set_titles([Some("mine.txt".into()), Some("theirs.txt".into())]);
        m.set_focus(Side::Right);
        crate::text::input::text(m, "edit ");
    }
    app.execute(CommandId::CopyFileRight);
    world.settle(&mut app);
    let w = app.win();
    let prompt = app
        .window_mut(w)
        .and_then(|x| x.dialogs.top_mut())
        .and_then(|d| {
            d.as_any()
                .downcast_ref::<crate::dialog::Prompt>()
                .map(|p| p.message.clone())
        })
        .expect("the confirmation");
    assert!(prompt.contains("Copy mine.txt to the right"), "{prompt}");
    assert!(
        prompt.contains("Discard the edits of theirs.txt"),
        "{prompt}"
    );
}
