//! A11.8 (D-301): the unsaved-edits prompts of every path that drops a modified document — Close Tab, closing a
//! window, Exit over two windows (window by window), Go to Home, the sessions combo, a workspace load, the IPC
//! `CloseView` and `Shutdown` with and without `force` — once per document, in the order of the views; the
//! auto-saved session of a modified view holds no edits (a file golden).

use std::path::Path;

use hikasa_core::Side;
use hikasa_session::sessions::{Session, SessionName, SessionRef, SessionType};

use crate::app::{Effect, UiMsg};
use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::tests_text::Fixture;
use crate::window::{CommandArg, Invocation};

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("HIKASA_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("mkdir");
        std::fs::write(&path, actual).expect("bless");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (bless with HIKASA_BLESS=1)", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "{name}");
}

/// A view with both panes modified.
fn modified() -> Fixture {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.place(Side::Right, 0, 0);
    f.type_text("y");
    f
}

#[derive(Debug, Clone, Copy)]
enum Way {
    Tab,
    Window,
    Exit,
    Home,
    Combo,
    Load,
    IpcClose,
    IpcQuit,
}

const WAYS: [Way; 8] = [
    Way::Tab,
    Way::Window,
    Way::Exit,
    Way::Home,
    Way::Combo,
    Way::Load,
    Way::IpcClose,
    Way::IpcQuit,
];

fn run_way(f: &mut Fixture, way: Way) -> Vec<Effect> {
    let w = f.app.win();
    let effects = match way {
        Way::Tab => f.app.0.execute(w, CommandId::CloseTab),
        Way::Window => {
            f.app.0.execute(w, CommandId::WindowNew);
            f.app.0.focus(w);
            f.app.0.close_window(w)
        }
        Way::Exit => f.app.0.execute(w, CommandId::Exit),
        Way::Home => f.app.0.execute(w, CommandId::SessionHome),
        Way::Combo => {
            let mut s = Session::builtin(SessionType::TextCompare);
            s.specs.left = f.paths[0].display().to_string();
            s.specs.right = f.paths[1].display().to_string();
            let other = SessionRef::parse("other").expect("ref");
            f.world
                .service
                .session_save(&other, &s, false)
                .expect("save");
            f.app.0.execute(
                w,
                Invocation::with(CommandId::SessionsCombo, CommandArg::Session(other)),
            )
        }
        Way::Load => {
            f.world
                .write("config/workspaces/empty.toml", "schema_version = 1\n");
            f.app.0.execute(
                w,
                Invocation::with(
                    CommandId::WorkspaceLoadList,
                    CommandArg::Workspace(SessionName::parse("empty").expect("name")),
                ),
            )
        }
        Way::IpcClose => {
            let view = f.view;
            f.app.handle(UiMsg::Close { view, force: false })
        }
        Way::IpcQuit => f.app.handle(UiMsg::Quit { force: false }),
    };
    f.settle();
    effects
}

/// Every path asks once per modified document, left then right; Cancel at the second keeps everything.
#[test]
fn a11_8_every_path_asks_once_per_document_in_order() {
    for way in WAYS {
        let mut f = modified();
        run_way(&mut f, way);
        assert_eq!(
            f.prompt().as_deref(),
            Some("Save changes to left.c?"),
            "{way:?}"
        );
        f.answer(ButtonRole::No);
        assert_eq!(
            f.prompt().as_deref(),
            Some("Save changes to right.c?"),
            "{way:?}: the second document"
        );
        f.answer(ButtonRole::Cancel);
        assert!(
            f.app.tab(f.view).is_some(),
            "{way:?}: Cancel keeps the view"
        );
        assert_eq!(f.text(Side::Left), "xabc\n", "{way:?}");
        assert_eq!(f.text(Side::Right), "yabc\n", "{way:?}");
        // The next try asks again, and Discard All lets it happen.
        run_way(&mut f, way);
        assert!(f.prompt().is_some(), "{way:?}: asked again");
        f.answer(crate::app::text::DISCARD_ALL);
        assert!(f.app.tab(f.view).is_none(), "{way:?}: closed");
        assert_eq!(
            std::fs::read(&f.paths[0]).expect("read"),
            b"abc\n",
            "{way:?}"
        );
    }
}

/// The IPC close and quit with `force` drop the edits without asking; an unmodified view never asks.
#[test]
fn a11_8_ipc_force_drops_the_edits() {
    let mut f = modified();
    let view = f.view;
    f.app.handle(UiMsg::Close { view, force: true });
    f.settle();
    assert!(f.prompt().is_none());
    assert!(f.app.tab(view).is_none());

    let mut f = modified();
    let effects = f.app.handle(UiMsg::Quit { force: true });
    assert!(effects.contains(&Effect::Exit), "{effects:?}");

    let mut f = Fixture::new("abc\n", "abc\n");
    let view = f.view;
    f.app.handle(UiMsg::Close { view, force: false });
    f.settle();
    assert!(f.prompt().is_none(), "nothing to ask");
    assert!(f.app.tab(view).is_none());
}

/// Exit over two windows: the views of the first window ask first, then those of the second; the last answer quits.
#[test]
fn a11_8_exit_asks_window_by_window() {
    let mut f = modified();
    let first = f.app.win();
    let second = f.world.write("work/second.c", "two\n");
    f.app.0.execute(first, CommandId::WindowNew);
    let w2 = f.app.win();
    assert_ne!(w2, first);
    let other = f.world.open(&mut f.app.0, &second, &f.paths[1].clone());
    f.settle();
    {
        let m = crate::tests::text_model(&mut f.app, other);
        m.set_focus(Side::Left);
        m.pane_mut(Side::Left)
            .set_caret(hikasa_text::edit::TextPos::ZERO, false);
        crate::text::input::text(m, "z");
    }
    let mut asked = Vec::new();
    f.app.0.focus(first);
    f.app.0.execute(first, CommandId::Exit);
    f.settle();
    let mut quit = false;
    for _ in 0..4 {
        let Some(p) = f.prompt() else {
            break;
        };
        if p.starts_with("Close ") {
            // The multiple-windows warning (D-106).
            f.answer(ButtonRole::Yes);
            continue;
        }
        asked.push((f.app.win(), p));
        let win = f.app.win();
        let service = f.world.service.clone();
        let step = {
            let w = f.app.window_mut(win).expect("window");
            let top = w.dialogs.top_mut().expect("a dialog");
            let p = top
                .as_any_mut()
                .downcast_mut::<crate::dialog::Prompt>()
                .expect("a prompt");
            crate::dialog::DialogModel::on_button(p, ButtonRole::No, &service)
        };
        let effects = f.app.dialog_step_pub(win, step);
        quit |= effects.contains(&Effect::Exit);
        f.settle();
    }
    let names: Vec<&str> = asked.iter().map(|(_, p)| p.as_str()).collect();
    assert_eq!(
        names,
        [
            "Save changes to left.c?",
            "Save changes to right.c?",
            "Save changes to second.c?"
        ]
    );
    assert_eq!(asked[0].0, first);
    assert_eq!(asked[2].0, w2, "the second window's view in its window");
    assert!(quit, "the last answer quits");
}

/// D-301: the auto-saved session of a modified view stores the session, never the edits (a file golden).
#[test]
fn a11_8_the_autosaved_session_holds_no_edits() {
    let mut f = modified();
    f.app.0.set_now(10.0);
    f.app.handle(UiMsg::Tick);
    f.app.0.set_now(400.0);
    f.app.handle(UiMsg::Tick);
    f.settle();
    let list = f
        .world
        .service
        .session_list(None, true, false)
        .expect("list")
        .autosaved;
    assert_eq!(list.len(), 1, "one auto-saved session");
    let id = hikasa_session::sessions::AutoId::parse(&list[0].id).expect("id");
    let path = f.world.service.session_store().autosaved_path(&id);
    let text = std::fs::read_to_string(path).expect("read");
    assert!(!text.contains("xabc") && !text.contains("yabc"), "{text}");
    let root = f.paths[0].parent().expect("work").display().to_string();
    let text: String = text
        .replace(&root, "<work>")
        .replace('\\', "/")
        .lines()
        .map(|l| {
            if l.starts_with("saved_at = ") {
                "saved_at = \"<time>\"\n".to_owned()
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    golden("text-autosave-modified.toml", &text);
}
