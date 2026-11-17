//! A11.15 (D-308): Find in Files. The dialog's request equals `text find`'s for the same words, and the panel's files and
//! hits equal `text find --json`'s after the stream, in the stream's order; Stop cancels; opening a hit opens or
//! activates a Text Edit view at its line and column (a Text Compare tab showing the file is activated instead); Replace
//! in Files equals `text find --replace --yes` on a twin folder, leaves out a file open with unsaved edits and reloads an
//! open unmodified view; the panel's heap with 100,000 hits.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{JobKind, NullProgress, Side};
use hikasa_session::api::{NoEvents, SaveAnswers, TextFindReplace, TextFindRequest};
use hikasa_text::edit::{TextPos, doc_end};

use crate::commands::{CommandId, Scope};
use crate::dialog::ButtonRole;
use crate::tests::{TestApp, World, text_model};
use crate::text::find_files::{FindFilesDialog, FindFilesPanel, ListRow};
use crate::window::ViewModel;

/// A folder of files with hits of `needle` (and a binary file and a file without one).
fn tree(world: &World, root: &str) {
    world.write(&format!("{root}/a.txt"), "one needle\ntwo\nneedle needle\n");
    world.write(&format!("{root}/sub/b.c"), "int needle = 1;\n");
    world.write(&format!("{root}/sub/c.md"), "nothing here\n");
    let bin = world.path(&format!("{root}/d.bin"));
    std::fs::write(bin, [0u8, 1, 2, b'n', b'e', b'e', b'd', b'l', b'e', 0]).expect("write");
}

/// The window's panel after `request`, settled.
fn search(world: &World, app: &mut TestApp, request: TextFindRequest) {
    let win = app.win();
    app.0.start_find_files_for_test(win, request);
    let began = Instant::now();
    loop {
        world.settle(app);
        let done = app
            .window_mut(win)
            .is_some_and(|w| !w.find_files.running() && w.find_files.summary.is_some());
        if done {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "the search did not end"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn panel(app: &mut TestApp) -> &mut FindFilesPanel {
    let win = app.win();
    &mut app.window_mut(win).expect("window").find_files
}

#[test]
fn a11_15_the_dialog_and_the_panel_equal_text_find() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    let mut d = FindFilesDialog::new(
        app.0.state(),
        vec![world.path("w").display().to_string()],
        "needle".into(),
    );
    d.masks = "*.txt;*.c".into();
    d.whole_word = true;
    let request = d.request(&world.service).expect("a request");
    // `text find needle <w> --recursive --word --mask "*.txt;*.c"`: the same words.
    let cli = TextFindRequest {
        pattern: "needle".into(),
        roots: vec![world.path("w").into_os_string()],
        whole_word: true,
        recursive: true,
        mask: Some("*.txt;*.c".into()),
        max_matches: Some(100_000),
        max_matches_per_file: Some(10_000),
        max_file_size: Some(128 << 20),
        ..TextFindRequest::default()
    };
    assert_eq!(request, cli);
    search(&world, &mut app, request.clone());
    let job = world.service.job(JobKind::TextFind, Arc::new(NullProgress));
    let json = world
        .service
        .text_find(
            &TextFindRequest {
                keep_hits: true,
                ..cli
            },
            &mut NoEvents,
            &job,
        )
        .expect("text find");
    let p = panel(&mut app);
    let files: Vec<(String, u32)> = p.files.iter().map(|f| (f.spec.clone(), f.hits)).collect();
    let expected: Vec<(String, u32)> = json
        .dto
        .files
        .iter()
        .map(|f| (f.spec.clone(), f.hit_count as u32))
        .collect();
    assert_eq!(files, expected, "the stream's order");
    let mut k = 0;
    for f in &json.dto.files {
        for h in &f.hits {
            let got = p.hits[k];
            assert_eq!((got.line, got.column, got.len), (h.line, h.column, h.len));
            assert_eq!(p.preview(k), h.preview);
            assert_eq!(got.range, (h.preview_range[0], h.preview_range[1]));
            k += 1;
        }
    }
    assert_eq!(k, p.hits.len());
    let summary = p.summary.clone().expect("summary");
    assert_eq!(summary.hits, json.dto.counts.hits);
    assert_eq!(summary.files_searched, json.dto.counts.files_searched);
    // The rows: each file header then its hits; Copy of a hit is `<spec>:<line>:<column>: <preview>`.
    assert_eq!(p.row(0), Some(ListRow::File(0)));
    assert_eq!(p.row(1), Some(ListRow::Hit(0)));
    let header_1 = 1 + p.files[0].hits as usize;
    assert_eq!(p.row(header_1), Some(ListRow::File(1)));
    p.selected = Some(1);
    let h = p.hits[0];
    assert_eq!(
        p.copy_text(),
        Some(format!(
            "{}:{}:{}: {}",
            p.files[0].spec,
            h.line,
            h.column,
            p.preview(0)
        ))
    );
    // The dialog's choices are remembered.
    assert!(d.remembered().contains(&("masks", "*.txt;*.c".to_owned())));
}

#[test]
fn a11_15_stop_cancels() {
    let world = World::new();
    for i in 0..400 {
        world.write(&format!("big/f{i}.txt"), &"needle haystack\n".repeat(2_000));
    }
    let mut app = world.app(None);
    let request = TextFindRequest {
        pattern: "needle".into(),
        roots: vec![world.path("big").into_os_string()],
        recursive: true,
        ..TextFindRequest::default()
    };
    let win = app.win();
    app.0.start_find_files_for_test(win, request);
    std::thread::sleep(Duration::from_millis(20));
    let stopped = Instant::now();
    panel(&mut app).stop();
    loop {
        world.settle(&mut app);
        if !panel(&mut app).running() {
            break;
        }
        assert!(stopped.elapsed() < Duration::from_secs(10));
    }
    eprintln!("Stop took {:?}", stopped.elapsed());
    let p = panel(&mut app);
    assert!(p.summary.is_none() || p.summary.as_ref().is_some_and(|s| s.files_searched < 400));
    assert!(p.summary_text().contains("Stopped") || p.summary.is_some());
}

#[test]
fn a11_15_opening_a_hit() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    search(
        &world,
        &mut app,
        TextFindRequest {
            pattern: "needle".into(),
            roots: vec![world.path("w").into_os_string()],
            recursive: true,
            ..TextFindRequest::default()
        },
    );
    // The third hit of a.txt: line 3, column 8.
    let k = 2;
    let (spec, line, column, _) = panel(&mut app).hit_target(k).expect("a hit");
    assert!(spec.ends_with("a.txt"));
    assert_eq!((line, column), (3, 8));
    let win = app.win();
    app.0.open_find_hit(win, k);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::TextEdit);
    let view = app.active_tab().map(|t| t.id).expect("tab");
    let m = text_model(&mut app, view);
    assert_eq!(
        m.pane(Side::Left).selection().range(),
        TextPos::new(2, 7)..TextPos::new(2, 13)
    );
    assert_eq!(m.find.query.text, "needle");
    // Again: the tab showing it is activated (no second view).
    let tabs = app.tabs().len();
    app.activate(0);
    app.0.open_find_hit(win, 0);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs);
    assert_eq!(app.active_tab().map(|t| t.id), Some(view));
    // A Text Compare tab showing the file in a pane is activated instead.
    let other = world.write("other.txt", "x\n");
    let a = world.path("w/a.txt");
    let compare = world.open(&mut app, &other, &a);
    world.settle(&mut app);
    app.activate(0);
    app.0.open_find_hit(win, 0);
    world.settle(&mut app);
    assert_eq!(app.active_tab().map(|t| t.id), Some(compare));
    assert_eq!(text_model(&mut app, compare).focus(), Side::Right);
}

/// The files of a folder, relative, with their contents (backups and the like included).
fn contents(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).expect("read_dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).expect("rel").display().to_string();
                out.push((rel, std::fs::read(&p).expect("read")));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a11_15_replace_in_files_equals_text_find_replace() {
    let world = World::new();
    tree(&world, "gui");
    tree(&world, "cli");
    let mut app = world.app(None);
    // `text find needle <cli> --recursive --replace pin --yes`.
    let job = world.service.job(JobKind::TextFind, Arc::new(NullProgress));
    let cli = TextFindRequest {
        pattern: "needle".into(),
        roots: vec![world.path("cli").into_os_string()],
        recursive: true,
        replace: Some(TextFindReplace {
            text: "pin".into(),
            answers: SaveAnswers {
                yes: true,
                ..SaveAnswers::default()
            },
            ..TextFindReplace::default()
        }),
        ..TextFindRequest::default()
    };
    let plan = world
        .service
        .text_find_replace_plan(&cli, &mut NoEvents, &job)
        .expect("plan");
    world
        .service
        .text_find_replace_execute(plan, &job)
        .expect("execute");
    // The window: an open unmodified view of a.txt (reloaded after), and b.c open with edits (left out).
    let a = world.path("gui/a.txt");
    let b = world.path("gui/sub/b.c");
    let other = world.write("x.txt", "x\n");
    let va = world.open(&mut app, &a, &other);
    world.settle(&mut app);
    let vb = world.open(&mut app, &b, &other);
    world.settle(&mut app);
    {
        let m = text_model(&mut app, vb);
        m.set_focus(Side::Left);
        m.pane_mut(Side::Left).set_caret(TextPos::ZERO, false);
        crate::text::input::text(m, "/* edited */ ");
    }
    let win = app.win();
    app.0.replace_in_files(
        win,
        TextFindRequest {
            pattern: "needle".into(),
            roots: vec![world.path("gui").into_os_string()],
            recursive: true,
            replace: Some(TextFindReplace {
                text: "pin".into(),
                ..TextFindReplace::default()
            }),
            ..TextFindRequest::default()
        },
    );
    world.settle(&mut app);
    // The preview lists the files (b.c with *open with unsaved edits*); the questions are answered Yes.
    let preview = panel(&mut app).preview.join("\n");
    assert!(preview.contains("open with unsaved edits"), "{preview}");
    let mut asked = 0;
    while let Some(text) = top_prompt(&mut app) {
        asked += 1;
        assert!(asked < 10, "{text}");
        answer(&world, &mut app, ButtonRole::Yes);
    }
    assert!(asked >= 1);
    world.settle(&mut app);
    // a.txt as the CLI wrote it; b.c not touched; the view of a.txt reloaded.
    assert_eq!(
        std::fs::read(&a).expect("read"),
        std::fs::read(world.path("cli/a.txt")).expect("read")
    );
    assert_eq!(
        std::fs::read_to_string(&b).expect("read"),
        "int needle = 1;\n"
    );
    let shown = {
        let m = text_model(&mut app, va);
        let d = m.doc(Side::Left).expect("doc").clone();
        d.text(TextPos::ZERO..doc_end(&d))
    };
    assert_eq!(shown, std::fs::read_to_string(&a).expect("read"));
    let gui_files: Vec<String> = contents(&world.path("gui"))
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let cli_files: Vec<String> = contents(&world.path("cli"))
        .into_iter()
        .map(|(p, _)| p)
        .filter(|p| p != "sub/b.c.bak")
        .collect();
    assert_eq!(
        gui_files, cli_files,
        "the same files and backups, b.c left out"
    );
    let _ = ViewModel::Home;
}

/// The message of the top prompt.
fn top_prompt(app: &mut TestApp) -> Option<String> {
    let win = app.win();
    let w = app.window_mut(win)?;
    let top = w.dialogs.top_mut()?;
    top.as_any()
        .downcast_ref::<crate::dialog::Prompt>()
        .map(|p| p.message.clone())
}

/// Presses `role` on the top prompt.
fn answer(world: &World, app: &mut TestApp, role: ButtonRole) {
    let win = app.win();
    let service = world.service.clone();
    let step = {
        let w = app.window_mut(win).expect("window");
        let top = w.dialogs.top_mut().expect("a dialog");
        let p = top
            .as_any_mut()
            .downcast_mut::<crate::dialog::Prompt>()
            .expect("a prompt");
        crate::dialog::DialogModel::on_button(p, role, &service)
    };
    app.0.dialog_step_pub(win, step);
    world.settle(app);
}

/// A searched file without hits is not listed, and each hit is listed under its own file.
#[test]
fn a11_15_files_without_hits_are_not_listed_and_hits_name_their_file() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    search(
        &world,
        &mut app,
        TextFindRequest {
            pattern: "needle".into(),
            roots: vec![world.path("w").into_os_string()],
            recursive: true,
            ..TextFindRequest::default()
        },
    );
    let p = panel(&mut app);
    let listed: Vec<(String, u32)> = p
        .files
        .iter()
        .map(|f| (crate::shared::spec_name(&f.spec).to_owned(), f.hits))
        .collect();
    assert_eq!(
        listed,
        vec![("a.txt".to_owned(), 3), ("b.c".to_owned(), 1)],
        "c.md (no hit) is not listed"
    );
    let file_of = |k: usize| {
        p.hit_target(k)
            .map(|t| crate::shared::spec_name(&t.0).to_owned())
            .expect("a hit")
    };
    assert_eq!(file_of(2), "a.txt");
    assert_eq!(file_of(3), "b.c");
    assert_eq!(p.row(4), Some(ListRow::File(1)));
    assert_eq!(p.row(5), Some(ListRow::Hit(3)));
}

#[test]
fn a11_15_the_panels_heap_with_100000_hits() {
    let world = World::new();
    // 50 files of 2,000 lines with one 40-character line each hit: 100,000 hits.
    let line = "x needle aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
    for i in 0..50 {
        world.write(&format!("h/f{i}.txt"), &line.repeat(2_000));
    }
    let mut app = world.app(None);
    search(
        &world,
        &mut app,
        TextFindRequest {
            pattern: "needle".into(),
            roots: vec![world.path("h").into_os_string()],
            recursive: true,
            max_matches: Some(100_000),
            max_matches_per_file: Some(10_000),
            ..TextFindRequest::default()
        },
    );
    let p = panel(&mut app);
    assert_eq!(p.hits.len(), 100_000);
    let heap = p.heap_bytes();
    let bound = (16 << 20) + p.preview_bytes();
    eprintln!("the panel's heap: {heap} bytes for 100,000 hits (bound {bound})");
    assert!(heap <= bound, "{heap} > {bound}");
    // Every part is counted: at least the previews and the hits' entries.
    let floor = p.preview_bytes() + p.hits.len() * std::mem::size_of_val(&p.hits[0]);
    assert!(heap >= floor, "{heap} < {floor}");
    // The rows are found by a binary search: the last hit's row.
    let last = p.row_count() - 1;
    assert_eq!(p.row(last), Some(ListRow::Hit(99_999)));
    assert!(p.drains >= 1);
}

#[test]
fn a11_15_the_command_opens_the_dialog() {
    let world = World::new();
    let l = world.write("w/l.txt", "a\n");
    let r = world.write("w/r.txt", "b\n");
    let mut app = world.app(None);
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    text_model(&mut app, view).find.query.text = "a".into();
    app.execute(CommandId::FindInFiles);
    let win = app.win();
    let d = app
        .window_mut(win)
        .and_then(|w| w.dialogs.find_mut::<FindFilesDialog>())
        .expect("the dialog")
        .clone();
    assert_eq!(d.text, "a");
    assert_eq!(d.look_in, vec![world.path("w").display().to_string()]);
    assert_eq!(
        crate::dialog::DialogModel::help_topic(&d),
        "dialog-find-in-files"
    );
}

/// The panel's dragged height is kept (`state.dialogs."find-in-files".panel_height`) and taken by a panel's first
/// search.
#[test]
fn a11_15_the_panels_height_is_remembered() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    let request = TextFindRequest {
        pattern: "needle".into(),
        roots: vec![world.path("w").into_os_string()],
        recursive: true,
        ..TextFindRequest::default()
    };
    search(&world, &mut app, request.clone());
    let win = app.win();
    app.0
        .find_files_action(win, crate::ui::find_files::FilesAction::Height(240.0));
    assert_eq!(panel(&mut app).height, 240.0);
    assert_eq!(
        app.0.state().dialog_text(
            crate::text::find_files::DIALOG_ID,
            crate::text::find_files::PANEL_HEIGHT_KEY
        ),
        Some("240")
    );
    // A new panel (another window's) starts at that height.
    *panel(&mut app) = FindFilesPanel::default();
    search(&world, &mut app, request);
    assert_eq!(panel(&mut app).height, 240.0);
}

// ---- W11b review fixes ------------------------------------------------------------------------------------------

fn needle(world: &World, root: &str) -> TextFindRequest {
    TextFindRequest {
        pattern: "needle".into(),
        roots: vec![world.path(root).into_os_string()],
        recursive: true,
        ..TextFindRequest::default()
    }
}

/// 10b: a Text Compare tab whose header path was typed otherwise (`w/./a.txt`) shows the hit's file: it is activated.
#[test]
fn w11b_review_a_compare_tab_is_matched_by_canonical_spec() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    search(&world, &mut app, needle(&world, "w"));
    let other = world.write("other.txt", "x\n");
    let compare = world.open(&mut app, &other, &other);
    world.settle(&mut app);
    // The left header's path, typed with a `.` (the doc's spec is normalised; the view's input is as typed).
    let typed = format!("{}/./a.txt", world.path("w").display());
    text_model(&mut app, compare).request_reopen(Side::Left, Some(typed.clone().into()), None);
    world.settle(&mut app);
    assert_eq!(text_model(&mut app, compare).inputs()[0], typed);
    let (hit, _, _, _) = panel(&mut app).hit_target(0).expect("hit");
    assert_ne!(
        typed, hit,
        "fixture: the typed spec is not the canonical one"
    );
    let tabs = app.tabs().len();
    app.activate(0);
    let win = app.win();
    app.0.open_find_hit(win, 0);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs, "no Text Edit view");
    assert_eq!(app.active_tab().map(|t| t.id), Some(compare));
    assert_eq!(text_model(&mut app, compare).focus(), Side::Left);
}

/// 10c: a hit opened in a Text Edit view whose document was edited since the search lands on the hit's text (moved by
/// the edits); with the text gone, the caret goes to the line with a message.
#[test]
fn w11b_review_a_hit_in_an_edited_view_lands_on_its_text() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    search(&world, &mut app, needle(&world, "w"));
    let win = app.win();
    app.0.open_find_hit(win, 0);
    world.settle(&mut app);
    let view = app.active_tab().map(|t| t.id).expect("tab");
    {
        let m = text_model(&mut app, view);
        let req = hikasa_text::edit::EditRequest::Replace {
            range: TextPos::ZERO..TextPos::ZERO,
            text: "new first line\nxx ".into(),
            keep_eols: false,
        };
        assert!(crate::text::edit::edit(
            m,
            Side::Left,
            req,
            crate::text::edit::Origin::Single,
            None
        ));
    }
    // The third hit (line 3, column 8 on disk) is on line 4 now, at column 8.
    app.0.open_find_hit(win, 2);
    world.settle(&mut app);
    let m = text_model(&mut app, view);
    let sel = m.pane(Side::Left).selection().range();
    assert_eq!(sel, TextPos::new(3, 7)..TextPos::new(3, 13));
    // The first hit's text no longer anywhere near: the caret on its line and a message.
    {
        let m = text_model(&mut app, view);
        let d = m.doc(Side::Left).expect("doc").clone();
        let all = hikasa_text::edit::EditRequest::Replace {
            range: TextPos::ZERO..doc_end(&d),
            text: "a\nb\nc\nd\n".into(),
            keep_eols: false,
        };
        assert!(crate::text::edit::edit(
            m,
            Side::Left,
            all,
            crate::text::edit::Origin::Single,
            None
        ));
    }
    app.0.open_find_hit(win, 0);
    world.settle(&mut app);
    let m = text_model(&mut app, view);
    assert_eq!(
        m.pane(Side::Left).selection().range(),
        TextPos::new(0, 0)..TextPos::new(0, 0)
    );
    let message = app
        .tab(view)
        .and_then(|t| t.message.as_ref())
        .map(|m| m.text.clone());
    assert_eq!(
        message.as_deref(),
        Some("The text of the hit is no longer on line 1.")
    );
}

/// C34: a hit waiting for the document wins over *go to the first difference on load*.
#[test]
fn w11b_review_a_waiting_hit_wins_over_the_first_difference() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    search(&world, &mut app, needle(&world, "w"));
    let other = world.write("other.txt", "x\n");
    let a = world.path("w/a.txt");
    let compare = world.open(&mut app, &other, &a);
    world.settle(&mut app);
    let service = world.service.clone();
    {
        let m = text_model(&mut app, compare);
        let mut t = *m.tweaks();
        t.first_difference_on_load = true;
        m.set_tweaks(t);
        // Reloaded: the hit waits for the documents.
        m.execute(&service, CommandId::Reload);
        m.set_focus(Side::Right);
        m.pending_goto = Some(crate::text::Goto {
            line: 3,
            column: 8,
            len: 6,
            text: Some("needle".into()),
        });
    }
    world.settle(&mut app);
    let m = text_model(&mut app, compare);
    assert_eq!(m.focus(), Side::Right);
    assert_eq!(
        m.pane(Side::Right).selection().range(),
        TextPos::new(2, 7)..TextPos::new(2, 13)
    );
}

/// 10a: a file that got unsaved edits while Replace in Files asked its questions is skipped (*has unsaved edits*),
/// never rewritten under its view.
#[test]
fn w11b_review_replace_in_files_rechecks_unsaved_edits_before_writing() {
    let world = World::new();
    tree(&world, "w");
    let mut app = world.app(None);
    let b = world.path("w/sub/b.c");
    let other = world.write("x.txt", "x\n");
    let vb = world.open(&mut app, &b, &other);
    world.settle(&mut app);
    let win = app.win();
    let mut request = needle(&world, "w");
    request.replace = Some(TextFindReplace {
        text: "pin".into(),
        ..TextFindReplace::default()
    });
    app.0.replace_in_files(win, request);
    world.settle(&mut app);
    assert!(top_prompt(&mut app).is_some());
    // Edits typed while the question is open.
    {
        let m = text_model(&mut app, vb);
        m.set_focus(Side::Left);
        m.pane_mut(Side::Left).set_caret(TextPos::ZERO, false);
        crate::text::input::text(m, "/* edited */ ");
    }
    let mut asked = 0;
    while top_prompt(&mut app).is_some() {
        asked += 1;
        assert!(asked < 10);
        answer(&world, &mut app, ButtonRole::Yes);
    }
    world.settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(&b).expect("read"),
        "int needle = 1;\n",
        "b.c is not rewritten"
    );
    assert!(
        std::fs::read_to_string(world.path("w/a.txt"))
            .expect("read")
            .contains("pin"),
        "the other files are"
    );
}
