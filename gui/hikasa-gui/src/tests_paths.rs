//! M2 W9a acceptance A9.5 (GUI part): the path edits above the panes — the 150 ms debounce by frame time and the
//! generations, the completion list through `path_completions` (Up/Down, Tab on a folder goes on), Accept re-roots the
//! view and pushes the MRU only for a base that loads, a missing folder asks to be created (Yes creates it and runs,
//! No keeps the text with the message), a file where a folder is needed, Esc and Cancel.

use std::time::{Duration, Instant};

use hikasa_core::Side;
use hikasa_session::api::paths::{Completion, CompletionDto};
use hikasa_session::ipc::ViewId;
use hikasa_session::settings::state::MruList;

use crate::dialog::ButtonRole;
use crate::path_edit::{DEBOUNCE, PathEdit, PathKey, PathKeyOutcome};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{open, press, put, top_title};

fn item(name: &str, folder: bool) -> Completion {
    Completion {
        name: name.to_owned(),
        text: if folder {
            format!("/x/{name}/")
        } else {
            format!("/x/{name}")
        },
        folder,
        archive: false,
        lossy: false,
    }
}

#[test]
fn a9_5_the_debounce_and_the_generations() {
    let t0 = Instant::now();
    let ms = |n: u64| t0 + Duration::from_millis(n);
    let mut edit = PathEdit::new(Side::Left, "/x");
    assert_eq!(edit.due(ms(0)), Err(None), "nothing typed");
    edit.text = "/x/a".into();
    edit.edited(ms(0));
    let first = edit.generation;
    assert_eq!(edit.due(ms(100)), Err(Some(Duration::from_millis(50))));
    // Another key restarts the wait.
    edit.text = "/x/ab".into();
    edit.edited(ms(120));
    assert_eq!(edit.due(ms(200)), Err(Some(Duration::from_millis(70))));
    assert_eq!(edit.due(ms(120) + DEBOUNCE), Ok(first + 1));
    assert_eq!(edit.due(ms(1_000)), Err(None), "asked once per edit");
    // An answer for the older text is dropped; the current one opens the list.
    let dto = |items| CompletionDto {
        base: "/x/".into(),
        items,
        truncated: false,
    };
    assert!(!edit.answer(first, dto(vec![item("old", false)])));
    assert!(!edit.open);
    assert!(edit.answer(
        first + 1,
        dto(vec![item("abc", true), item("abd.txt", false)])
    ));
    assert!(edit.open);
    // Down, Down, Up, Tab: the folder's text with its separator, and the next completion is due.
    assert_eq!(edit.key(PathKey::Down, ms(1_000)), PathKeyOutcome::Handled);
    assert_eq!(edit.key(PathKey::Down, ms(1_000)), PathKeyOutcome::Handled);
    assert_eq!(edit.highlighted, Some(1));
    assert_eq!(edit.key(PathKey::Up, ms(1_000)), PathKeyOutcome::Handled);
    assert_eq!(edit.key(PathKey::Tab, ms(1_000)), PathKeyOutcome::Handled);
    assert_eq!(edit.text, "/x/abc/");
    assert!(!edit.open);
    assert!(edit.due(ms(1_000) + DEBOUNCE).is_ok());
    // Esc with the list closed: the committed spec again; Enter: Accept.
    assert!(edit.changed());
    assert_eq!(edit.key(PathKey::Esc, ms(2_000)), PathKeyOutcome::Cancelled);
    assert_eq!(edit.text, "/x");
    assert!(!edit.changed());
    assert_eq!(edit.key(PathKey::Enter, ms(2_000)), PathKeyOutcome::Accept);
}

/// Types `text` in path edit `k` and lets the completion run.
fn complete(world: &World, app: &mut TestApp, view: ViewId, k: usize, text: &str) {
    let t0 = Instant::now();
    let m = folder_model(app, view);
    m.path_edit(k).text = text.to_owned();
    m.path_edit(k).edited(t0);
    assert!(m.path_frame(t0).is_some());
    assert_eq!(m.path_frame(t0 + DEBOUNCE), None);
    let began = Instant::now();
    while folder_model(app, view).path_edit(k).completions.is_empty() {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
        assert!(began.elapsed() < Duration::from_secs(30), "no completion");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn committed(app: &mut TestApp, view: ViewId) -> [String; 2] {
    let m = folder_model(app, view);
    [
        m.path_edit(0).committed.clone(),
        m.path_edit(1).committed.clone(),
    ]
}

/// Accepts the fields and lets the check and the reopen run.
fn accept(world: &World, app: &mut TestApp, view: ViewId) {
    folder_model(app, view).accept_paths();
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(app);
}

#[test]
fn a9_5_completion_from_disk_and_accept_re_roots_the_view() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    put(&world, "other", "sub/b.txt", "b\n", 1_000);
    put(&world, "other", "sea.txt", "s\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let other = world.path("work/other").to_string_lossy().into_owned();
    complete(&world, &mut app, view, 0, &format!("{other}/s"));
    let m = folder_model(&mut app, view);
    let names: Vec<_> = m
        .path_edit(0)
        .completions
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["sub", "sea.txt"], "folders first");
    assert!(m.path_edit(0).open);
    let now = Instant::now();
    m.path_key(0, PathKey::Down, now);
    m.path_key(0, PathKey::Enter, now);
    let text = m.path_edit(0).text.clone();
    assert!(
        text.ends_with(std::path::MAIN_SEPARATOR) && text.contains("sub"),
        "{text}"
    );
    // Accept: the view runs on the new left base; it enters mru.left, the unchanged right does not.
    assert!(app.0.state().mru(MruList::Left).is_empty());
    accept(&world, &mut app, view);
    let [left, right] = committed(&mut app, view);
    assert!(left.contains("sub"), "{left}");
    assert!(right.ends_with('r'), "{right}");
    assert_eq!(
        app.0.state().mru(MruList::Left),
        std::slice::from_ref(&text)
    );
    assert!(app.0.state().mru(MruList::Right).is_empty());
    let m = folder_model(&mut app, view);
    assert!(
        m.rows()
            .iter()
            .any(|r| r.node != hikasa_folder::NodeId::ROOT),
        "it ran"
    );
}

#[test]
fn a9_5_a_missing_folder_asks_yes_creates_no_keeps_the_text() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    let missing = world.path("work/new/deeper").to_string_lossy().into_owned();
    folder_model(&mut app, view).path_edit(1).text = missing.clone();
    accept(&world, &mut app, view);
    assert_eq!(top_title(&app).as_deref(), Some("Create Folder"));
    // No: nothing created, the text stays with the message, the view unchanged, no MRU entry.
    press(&world, &mut app, ButtonRole::No);
    let m = folder_model(&mut app, view);
    assert_eq!(m.path_edit(1).text, missing);
    assert!(
        m.path_edit(1)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("does not exist"))
    );
    assert_eq!(committed(&mut app, view), before);
    assert!(!world.path("work/new").exists());
    assert!(app.0.state().mru(MruList::Right).is_empty());
    // Yes: created with its parents, then the run.
    accept(&world, &mut app, view);
    press(&world, &mut app, ButtonRole::Yes);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    assert!(world.path("work/new/deeper").is_dir());
    assert_eq!(committed(&mut app, view)[1], missing);
    assert_eq!(app.0.state().mru(MruList::Right), [missing]);
}

#[test]
fn a9_5_a_file_or_a_bad_spec_shows_its_message_and_does_not_run() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    let file = world.path("work/l/a.txt").to_string_lossy().into_owned();
    folder_model(&mut app, view).path_edit(0).text = file;
    accept(&world, &mut app, view);
    assert!(top_title(&app).is_none());
    let m = folder_model(&mut app, view);
    assert!(
        m.path_edit(0)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("not a folder")),
        "{:?}",
        m.path_edit(0).error
    );
    assert_eq!(committed(&mut app, view), before);
    // Cancel: the committed text again, the message gone.
    let m = folder_model(&mut app, view);
    m.path_edit(0).cancel();
    assert_eq!(m.path_edit(0).text, before[0]);
    assert!(m.path_edit(0).error.is_none());
    // An empty field.
    m.path_edit(0).text.clear();
    accept(&world, &mut app, view);
    assert!(folder_model(&mut app, view).path_edit(0).error.is_some());
    assert_eq!(committed(&mut app, view), before);
}

// ---- after the W9a review ------------------------------------------------------------------------------------------

fn lossy_item() -> Completion {
    Completion {
        name: "bad\u{fffd}.txt".into(),
        text: String::new(),
        folder: false,
        archive: false,
        lossy: true,
    }
}

/// Mutation O and review item 13: Esc with the list open closes the list and keeps the text; Tab with nothing
/// highlighted is not the field's (the focus moves on); a lossy item is never taken (finding 12).
#[test]
fn esc_tab_and_a_lossy_item() {
    let now = Instant::now();
    let mut edit = PathEdit::new(Side::Left, "/x");
    edit.text = "/x/ab".into();
    edit.edited(now);
    let generation = edit.generation;
    let dto = CompletionDto {
        base: "/x/".into(),
        items: vec![item("abc", true), lossy_item()],
        truncated: false,
    };
    assert!(edit.answer(generation, dto));
    assert!(edit.open);
    assert!(
        !edit.handles(PathKey::Tab),
        "nothing highlighted: Tab moves the focus"
    );
    assert_eq!(edit.key(PathKey::Esc, now), PathKeyOutcome::Handled);
    assert!(!edit.open, "the list closed");
    assert_eq!(edit.text, "/x/ab", "the text stays");
    // The list again; the lossy item highlighted: Tab takes nothing and says why.
    edit.open = true;
    edit.highlighted = Some(1);
    assert!(edit.handles(PathKey::Tab));
    assert_eq!(edit.key(PathKey::Tab, now), PathKeyOutcome::Handled);
    assert_eq!(edit.text, "/x/ab", "no text with a replacement character");
    assert_eq!(edit.error.as_deref(), Some(crate::path_edit::LOSSY_ITEM));
}

/// Review item 13: a key cancels the running completion job (its answer would be dropped anyway).
#[test]
fn a_key_cancels_the_running_completion() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let t0 = Instant::now();
    let other = world.path("work/l").to_string_lossy().into_owned();
    let m = folder_model(&mut app, view);
    m.path_edit(0).text = format!("{other}/");
    m.path_edit(0).edited(t0);
    assert_eq!(m.path_frame(t0 + DEBOUNCE), None);
    app.0.handle(crate::app::UiMsg::Tick);
    let m = folder_model(&mut app, view);
    let asked = m.completion_job().map(|(_, g)| g);
    if asked.is_some() {
        m.path_edit(0).text = format!("{other}/a");
        m.path_edit(0).edited(t0 + DEBOUNCE);
        let _ = m.path_frame(t0 + DEBOUNCE);
        assert_eq!(m.completion_job(), None, "the key cancelled it");
    }
    // Either way the next completion answers the newest text only.
    let _ = m.path_frame(t0 + DEBOUNCE * 3);
    let began = Instant::now();
    while folder_model(&mut app, view)
        .path_edit(0)
        .completions
        .is_empty()
    {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
        assert!(began.elapsed() < Duration::from_secs(30), "no completion");
        std::thread::sleep(Duration::from_millis(2));
    }
    let names: Vec<String> = folder_model(&mut app, view)
        .path_edit(0)
        .completions
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["a.txt"]);
}

/// Finding 9 and item 13: `-` in the right field is refused under the right field; nothing reads standard input.
#[test]
fn a_typed_input_spec_is_refused_under_its_field() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    folder_model(&mut app, view).path_edit(1).text = "-".into();
    accept(&world, &mut app, view);
    let m = folder_model(&mut app, view);
    assert_eq!(
        m.path_edit(1).error.as_deref(),
        Some(hikasa_session::api::paths::INPUT_SPEC_REFUSED)
    );
    assert_eq!(m.path_edit(0).error, None, "not under the left field");
    assert_eq!(committed(&mut app, view), before);
}

/// A9.5: `max_mru_items` caps the list a base enters.
#[test]
fn a9_5_mru_capped_at_max_mru_items() {
    let world = World::with_files(&[("options.toml", "[tweaks.metrics]\nmax_mru_items = 2\n")]);
    put(&world, "l", "a.txt", "a\n", 1_000);
    for name in ["one", "two", "three"] {
        put(&world, name, "x.txt", "x\n", 1_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    for name in ["one", "two", "three"] {
        let spec = world
            .path(&format!("work/{name}"))
            .to_string_lossy()
            .into_owned();
        folder_model(&mut app, view).path_edit(0).text = spec;
        accept(&world, &mut app, view);
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
    }
    let spec = |name: &str| {
        world
            .path(&format!("work/{name}"))
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        app.0.state().mru(MruList::Left),
        [spec("three"), spec("two")]
    );
}

/// Review item 13: a base enters the recent list once the view runs on it, not when its check passed: a re-root
/// refused because an operation started meanwhile leaves the list as it was.
#[test]
fn the_mru_waits_for_the_base_to_load() {
    use crate::tests_folder_ops::{gate, poll_until, select, set_gate};
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    put(&world, "other", "b.txt", "b\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let other = world.path("work/other").to_string_lossy().into_owned();
    folder_model(&mut app, view).path_edit(0).text = other;
    folder_model(&mut app, view).accept_paths();
    // The check passes; its reopen is still to come.
    poll_until(&world, &mut app, |a| a.mru_after_load.contains_key(&view));
    assert!(
        app.0.state().mru(MruList::Left).is_empty(),
        "not before the base loads"
    );
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(std::sync::Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(crate::commands::CommandId::CopyRight);
    poll_until(&world, &mut app, |a| !a.mru_after_load.contains_key(&view));
    assert!(
        app.0.state().mru(MruList::Left).is_empty(),
        "the re-root was refused"
    );
    set_gate(&g, true);
    world.settle(&mut app);
}
