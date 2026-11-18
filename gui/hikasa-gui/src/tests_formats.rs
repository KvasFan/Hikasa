//! M2 W11c acceptance A11.24 (D-318) and A11.25 (D-319, D-320), through the models: the File Formats manager's list
//! equals `format list` (order, shadowing, origins, *Everything Else* last); each button's effect on the configuration
//! folder equals the `hikasa format …` command on a twin folder (files compared); *Delete* is refused for built-ins;
//! *Restore* shows its plan first; a change on disk refreshes the list and a written format gives open views *Reopen*.
//! The format editor: an edit through each tab of each kind written by `format_save` equals `format edit --set` of the
//! same keys; unknown keys kept; a conflicting change asks *Overwrite* / *Reload* / *Cancel*; the Grammar Item dialog's
//! fields to the model and back; `format_check`'s message names the item; the preview equals `format_preview`; the regex
//! helper inserts each item at the caret in every regex field.

use std::collections::BTreeMap;

use hikasa_formats::{FormatKind, GrammarItem, ItemKind};
use hikasa_session::api::MoveTo;

use crate::browse::BrowseTarget;
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel as _, DialogStep, Prompt};
use crate::formats::editor::{ConversionChoice, EditorTab, FormatEditor};
use crate::formats::grammar_item::{Category, GrammarItemDialog};
use crate::formats::{FileFormatManager, RowOrigin, helper_insert};
use crate::tests::{TestApp, World};
use crate::tests_folder_ops::notices;
use crate::tests_text::Fixture;

/// A started window (its first-run files written) of a fresh configuration.
fn started() -> (World, TestApp) {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app);
    (world, app)
}

/// Every file below `config/formats`: name → content.
fn formats_dir(world: &World) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(read) = std::fs::read_dir(world.path("config/formats")) {
        for e in read.flatten() {
            out.insert(
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap_or_default(),
            );
        }
    }
    out
}

fn manager(app: &mut TestApp) -> &mut FileFormatManager {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<FileFormatManager>())
        .expect("the manager")
}

fn editor(app: &mut TestApp) -> &mut FormatEditor {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<FormatEditor>())
        .expect("an editor")
}

fn top_prompt(app: &TestApp) -> Option<Prompt> {
    let top = app.window(app.win())?.dialogs.top()?;
    top.as_any().downcast_ref::<Prompt>().cloned()
}

fn press(world: &World, app: &mut TestApp, role: ButtonRole) {
    let w = app.win();
    app.0.dialog_button(w, role);
    world.settle(app);
}

fn open_manager(world: &World, app: &mut TestApp) {
    app.execute(CommandId::FileFormatManager);
    world.settle(app);
}

fn select(app: &mut TestApp, id: &str) {
    let m = manager(app);
    let k = m.rows.iter().position(|r| r.id == id).expect("the row");
    m.select(k);
}

#[test]
fn a11_24_the_list_equals_format_list() {
    let (world, mut app) = started();
    // A user format shadowed by C (its whole mask is C's), a disabled one, an override of a built-in.
    world
        .service
        .format_new("shadow", None, Some("Shadowed"), Some("*.c"), None)
        .unwrap();
    world.service.format_enable("pascal", false).unwrap();
    world
        .service
        .format_edit("java", &[("description".into(), "\"mine\"".into())], None)
        .unwrap();
    world.service.reload_settings();
    open_manager(&world, &mut app);
    let list = world.service.format_list().formats;
    let m = manager(&mut app);
    let ids: Vec<&str> = m.rows.iter().map(|r| r.id.as_str()).collect();
    let want: Vec<&str> = list
        .iter()
        .map(|f| f.id.as_str())
        .chain(std::iter::once(""))
        .collect();
    assert_eq!(ids, want, "detection order, Everything Else last");
    let last = m.rows.last().unwrap();
    assert!(last.fixed && last.name == "Everything Else");
    let row = |id: &str| m.rows.iter().find(|r| r.id == id).unwrap().clone();
    assert_eq!(row("shadow").origin, RowOrigin::User);
    assert!(
        row("shadow")
            .tooltip()
            .is_some_and(|t| t.contains("C, C++")),
        "{:?}",
        row("shadow").shadowed_by
    );
    assert!(!row("pascal").enabled);
    assert_eq!(row("java").origin, RowOrigin::Override);
    assert_eq!(row("c-cpp").origin, RowOrigin::BuiltIn);
    for (f, r) in list.iter().zip(&m.rows) {
        assert_eq!(
            (f.enabled, f.status.status == "shadowed"),
            (r.enabled, !r.shadowed_by.is_empty()),
            "{}",
            f.id
        );
    }
    assert_eq!(m.help_topic(), "dialog-file-formats");
}

/// One action of the manager and the CLI's command for it.
type Action = (&'static str, fn(&World, &mut TestApp), fn(&World));

fn actions() -> Vec<Action> {
    vec![
        (
            "disable",
            |_, app| {
                let k = manager(app)
                    .rows
                    .iter()
                    .position(|r| r.id == "bash")
                    .unwrap();
                manager(app).set_enabled(k, false);
            },
            |w| {
                w.service.format_enable("bash", false).unwrap();
            },
        ),
        (
            "move up",
            |_, app| {
                select(app, "java");
                manager(app).move_selected(MoveTo::Up);
            },
            |w| {
                w.service.format_move("java", &MoveTo::Up).unwrap();
            },
        ),
        (
            "move down",
            |_, app| {
                select(app, "java");
                manager(app).move_selected(MoveTo::Down);
            },
            |w| {
                w.service.format_move("java", &MoveTo::Down).unwrap();
            },
        ),
        (
            "move top",
            |_, app| {
                select(app, "rust");
                manager(app).move_selected(MoveTo::Top);
            },
            |w| {
                w.service.format_move("rust", &MoveTo::Top).unwrap();
            },
        ),
        (
            "move bottom",
            |_, app| {
                select(app, "bash");
                manager(app).move_selected(MoveTo::Bottom);
            },
            |w| {
                w.service.format_move("bash", &MoveTo::Bottom).unwrap();
            },
        ),
        (
            "new table",
            |_, app| {
                let m = manager(app);
                m.start_new(FormatKind::Table);
                let p = m.prompt.as_mut().unwrap();
                p.id = "tab-x".into();
                p.name = "Tab X".into();
                m.create();
            },
            |w| {
                w.service
                    .format_new("tab-x", None, Some("Tab X"), None, Some(FormatKind::Table))
                    .unwrap();
            },
        ),
        (
            "clone",
            |_, app| {
                select(app, "c-cpp");
                let m = manager(app);
                m.start_clone();
                m.prompt.as_mut().unwrap().id = "my-c".into();
                m.create();
            },
            |w| {
                w.service
                    .format_new(
                        "my-c",
                        Some("c-cpp"),
                        Some("C, C++, C#, ObjC Source (copy)"),
                        None,
                        None,
                    )
                    .unwrap();
            },
        ),
        (
            "import",
            |world, app| {
                let path = world.path("in/imp.toml");
                // The browse dialog's answer (D-240).
                let w = app.win();
                app.0.browsed(
                    w,
                    BrowseTarget::FormatImport,
                    Some((path.to_string_lossy().into_owned(), false)),
                    false,
                );
            },
            |w| {
                w.service
                    .format_import(&w.path("in/imp.toml"), false)
                    .unwrap();
            },
        ),
    ]
}

const IMPORTED: &str =
    "schema_version = 1\nid = \"imp\"\nname = \"Imported\"\nkind = \"text\"\nmask = \"*.imp\"\n";

#[test]
fn a11_24_each_button_equals_its_format_command() {
    for (what, gui, cli) in actions() {
        let (world, mut app) = started();
        let (twin, mut tapp) = started();
        for w in [&world, &twin] {
            w.write("in/imp.toml", IMPORTED);
        }
        open_manager(&world, &mut app);
        gui(&world, &mut app);
        world.settle(&mut app);
        cli(&twin);
        twin.settle(&mut tapp);
        assert_eq!(formats_dir(&world), formats_dir(&twin), "{what}");
        assert_eq!(manager(&mut app).error, None, "{what}");
        // The list follows the write.
        let ids: Vec<String> = manager(&mut app)
            .rows
            .iter()
            .filter(|r| !r.fixed)
            .map(|r| r.id.clone())
            .collect();
        let list: Vec<String> = world
            .service
            .format_list()
            .formats
            .into_iter()
            .map(|f| f.id)
            .collect();
        assert_eq!(ids, list, "{what}");
        if what == "new table" || what == "clone" {
            // Then the editor of the new format.
            let id = editor(&mut app).id.clone();
            assert_eq!(id, if what == "clone" { "my-c" } else { "tab-x" });
        }
    }
}

#[test]
fn a11_24_delete_export_import_replace_and_restore() {
    let (world, mut app) = started();
    let (twin, mut tapp) = started();
    for w in [&world, &twin] {
        w.service
            .format_new("mine", None, Some("Mine"), Some("*.mine"), None)
            .unwrap();
        w.service
            .format_edit("java", &[("mask".into(), "\"*.java;*.jav\"".into())], None)
            .unwrap();
        w.service.reload_settings();
    }
    twin.settle(&mut tapp);
    open_manager(&world, &mut app);
    // Delete of a built-in: refused, no question.
    select(&mut app, "c-cpp");
    manager(&mut app).ask_delete();
    world.settle(&mut app);
    assert!(
        manager(&mut app)
            .error
            .as_ref()
            .unwrap()
            .contains("built-in")
    );
    assert!(top_prompt(&app).is_none());
    // Delete of a user format: the question, then `format delete`.
    select(&mut app, "mine");
    manager(&mut app).ask_delete();
    world.settle(&mut app);
    assert!(top_prompt(&app).unwrap().message.contains("Mine"));
    press(&world, &mut app, ButtonRole::Yes);
    twin.service.format_delete("mine").unwrap();
    assert_eq!(formats_dir(&world), formats_dir(&twin), "delete");
    // Export of the selected format: `format export`.
    select(&mut app, "rust");
    let out = world.path("out/rust.toml");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    manager(&mut app).export(out.clone());
    world.settle(&mut app);
    let twin_out = twin.path("out.toml");
    twin.service.format_export("rust", Some(&twin_out)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        std::fs::read_to_string(&twin_out).unwrap()
    );
    // Import of an id that exists: Replace asked, then `format import --replace`.
    let file = world.write(
        "in/rust2.toml",
        &std::fs::read_to_string(&out)
            .unwrap()
            .replace("Rust", "Rust 2"),
    );
    manager(&mut app).import(file.clone(), false);
    world.settle(&mut app);
    assert!(top_prompt(&app).unwrap().message.contains("Replace"));
    press(&world, &mut app, ButtonRole::Yes);
    let twin_file = twin.write("in/rust2.toml", &std::fs::read_to_string(&file).unwrap());
    twin.service.format_import(&twin_file, true).unwrap();
    assert_eq!(formats_dir(&world), formats_dir(&twin), "import --replace");
    // Restore Built-in of the overridden java: the plan first; the file stays until Yes.
    select(&mut app, "java");
    assert!(manager(&mut app).can_restore());
    manager(&mut app).ask_restore(false);
    world.settle(&mut app);
    let plan = top_prompt(&app).expect("the plan");
    assert!(plan.message.contains("java.toml"), "{}", plan.message);
    assert!(world.path("config/formats/java.toml").is_file());
    press(&world, &mut app, ButtonRole::Yes);
    twin.service
        .format_restore(&["java".to_owned()], false, false)
        .unwrap();
    assert_eq!(formats_dir(&world), formats_dir(&twin), "restore");
    // Restore All: every user file and order.toml.
    manager(&mut app).ask_restore(true);
    world.settle(&mut app);
    assert!(top_prompt(&app).is_some());
    press(&world, &mut app, ButtonRole::Yes);
    twin.service.format_restore(&[], true, false).unwrap();
    assert_eq!(formats_dir(&world), formats_dir(&twin), "restore all");
    assert!(
        manager(&mut app)
            .rows
            .iter()
            .all(|r| r.origin == RowOrigin::BuiltIn)
    );
}

#[test]
fn a11_24_a_change_on_disk_refreshes_and_written_formats_offer_reopen() {
    let mut f = Fixture::new("int a;\n", "int b;\n");
    open_manager(&f.world, &mut f.app);
    let before = manager(&mut f.app).rows.len();
    // Another program adds a format; the watch reloads the registry: the list follows.
    f.world.write(
        "config/formats/ext.toml",
        "schema_version = 1\nid = \"ext\"\nname = \"External\"\nkind = \"text\"\nmask = \"*.ext\"\n",
    );
    f.world.service.reload_settings();
    f.settle();
    assert_eq!(manager(&mut f.app).rows.len(), before + 1);
    // A format the view uses, written by the manager: the view offers Reopen.
    let k = manager(&mut f.app)
        .rows
        .iter()
        .position(|r| r.id == "c-cpp")
        .unwrap();
    manager(&mut f.app).set_enabled(k, false);
    f.settle();
    let reopen = notices(&f.app).into_iter().any(|(_, text, actions)| {
        text == "File format C, C++, C#, ObjC Source changed on disk."
            && actions == vec![crate::notify::NoticeAction::FormatReopen(f.view)]
    });
    assert!(reopen, "{:?}", notices(&f.app));
}

/// An edit through a tab of a kind and the `format edit --set` keys of it.
type EditorEdit = (
    &'static str,
    &'static str,
    Vec<(&'static str, &'static str)>,
    fn(&mut FormatEditor),
);

fn editor_edits() -> Vec<EditorEdit> {
    vec![
        ("c-cpp", "General", vec![("mask", "\"*.c;*.h\"")], |e| {
            e.format.mask = "*.c;*.h".into()
        }),
        ("c-cpp", "General", vec![("name", "\"C family\"")], |e| {
            e.format.name = "C family".into()
        }),
        (
            "c-cpp",
            "Conversion",
            vec![("conversion.method", "\"sorted\"")],
            |e| e.set_conversion(ConversionChoice::Sorted),
        ),
        (
            "c-cpp",
            "Grammar",
            vec![("grammar.1.stop_at_eol", "true")],
            |e| {
                let mut item = e.format.grammar[1].clone();
                if let ItemKind::Delimited { stop_at_eol, .. } = &mut item.kind {
                    *stop_at_eol = true;
                }
                e.item_done(crate::formats::grammar_item::ItemDone {
                    index: Some(1),
                    item,
                    case_sensitive: true,
                });
            },
        ),
        (
            "c-cpp",
            "Grammar",
            vec![("elements.Number.important", "false")],
            |e| {
                let mut info = e.element_info("Number");
                info.important = false;
                e.set_element_info("Number", info);
            },
        ),
        ("c-cpp", "Misc", vec![("text.tab_size", "8")], |e| {
            e.typed.tab_size = "8".into()
        }),
        (
            "c-cpp",
            "Misc",
            vec![("text.save_trim_trailing_ws", "true")],
            |e| e.format.text.save_trim_trailing_ws = true,
        ),
        ("csv", "Type", vec![("table.delimiter", "\";\"")], |e| {
            e.typed.delimiter = ";".into()
        }),
        (
            "csv",
            "Regional",
            vec![("table.regional.date_order", "\"dmy\"")],
            |e| {
                e.format.table.as_mut().unwrap().regional.date_order =
                    hikasa_formats::DateOrderSetting::Dmy
            },
        ),
        ("pictures", "General", vec![("mask", "\"*.png\"")], |e| {
            e.format.mask = "*.png".into()
        }),
        (
            "pictures",
            "Conversion",
            vec![("conversion.method", "\"sorted\"")],
            |e| e.set_conversion(ConversionChoice::Sorted),
        ),
        ("media", "General", vec![("description", "\"m\"")], |e| {
            e.format.description = "m".into()
        }),
        ("executables", "General", vec![("mask", "\"*.exe\"")], |e| {
            e.format.mask = "*.exe".into()
        }),
        (
            "registry-files",
            "General",
            vec![("mask", "\"*.reg\"")],
            |e| e.format.mask = "*.reg".into(),
        ),
    ]
}

fn open_editor(world: &World, app: &mut TestApp, id: &str) {
    let w = app.win();
    app.0.open_format_editor(w, id);
    world.settle(app);
    assert!(editor(app).ready(), "the format is read");
}

#[test]
fn a11_25_an_edit_through_each_tab_of_each_kind_is_format_edit_set() {
    for (id, tab, keys, edit) in editor_edits() {
        let (world, mut app) = started();
        let (twin, mut tapp) = started();
        open_editor(&world, &mut app, id);
        let e = editor(&mut app);
        let caption = e.tabs().iter().map(|t| t.caption()).find(|c| *c == tab);
        assert_eq!(caption, Some(tab), "{id}: the kind has the tab");
        edit(e);
        press(&world, &mut app, ButtonRole::Ok);
        assert!(
            app.window(app.win()).unwrap().dialogs.is_empty(),
            "{id} {tab}: OK closed ({:?})",
            app.window(app.win()).unwrap().dialogs
        );
        let set: Vec<(String, String)> = keys
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        twin.service.format_edit(id, &set, None).unwrap();
        twin.settle(&mut tapp);
        assert_eq!(
            formats_dir(&world),
            formats_dir(&twin),
            "{id} {tab} {keys:?}"
        );
    }
}

#[test]
fn a11_25_the_tabs_of_each_kind() {
    use crate::formats::editor::tabs_of;
    use EditorTab as T;
    assert_eq!(
        tabs_of(FormatKind::Text),
        [T::General, T::Conversion, T::Grammar, T::Misc]
    );
    assert_eq!(
        tabs_of(FormatKind::Table),
        [T::General, T::Conversion, T::Type, T::Regional]
    );
    assert_eq!(tabs_of(FormatKind::Hex), [T::General, T::Conversion]);
    assert_eq!(tabs_of(FormatKind::Picture), [T::General, T::Conversion]);
    for k in [FormatKind::Media, FormatKind::Version, FormatKind::Registry] {
        assert_eq!(tabs_of(k), [T::General]);
    }
}

#[test]
fn a11_25_unknown_keys_kept_and_a_conflict_asks() {
    let (world, mut app) = started();
    world
        .service
        .format_new("mine", None, Some("Mine"), Some("*.mine"), None)
        .unwrap();
    let path = world.path("config/formats/mine.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}future_key = 1\n")).unwrap();
    world.service.reload_settings();
    open_editor(&world, &mut app, "mine");
    editor(&mut app).format.description = "first-desc".into();
    press(&world, &mut app, ButtonRole::Ok);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("future_key = 1") && saved.contains("first-desc"),
        "{saved}"
    );
    // Changed on disk since the editor read it: Overwrite / Reload / Cancel.
    for answer in [
        ButtonRole::Cancel,
        ButtonRole::Other(2),
        ButtonRole::Other(1),
    ] {
        open_editor(&world, &mut app, "mine");
        editor(&mut app).format.description = "mine now".into();
        let theirs = std::fs::read_to_string(&path)
            .unwrap()
            .replace("first-desc", "theirs");
        std::fs::write(&path, &theirs).unwrap();
        press(&world, &mut app, ButtonRole::Ok);
        let error = editor(&mut app).error.clone();
        let p = top_prompt(&app).unwrap_or_else(|| {
            panic!(
                "the conflict: {:?} {error:?}",
                app.window(app.win()).unwrap().dialogs
            )
        });
        assert_eq!(p.title, "Format Changed on Disk");
        let labels: Vec<&str> = p.buttons.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["Overwrite", "Reload (discard my edits)", "Cancel"]);
        press(&world, &mut app, answer);
        let now = std::fs::read_to_string(&path).unwrap();
        match answer {
            ButtonRole::Cancel => {
                assert_eq!(now, theirs);
                assert_eq!(editor(&mut app).format.description, "mine now");
            }
            ButtonRole::Other(2) => {
                assert_eq!(now, theirs);
                assert_eq!(editor(&mut app).format.description, "theirs");
            }
            _ => {
                assert!(now.contains("mine now"), "{now}");
                assert!(app.window(app.win()).unwrap().dialogs.is_empty());
            }
        }
        // Back to a known state for the next answer.
        while !app.window(app.win()).unwrap().dialogs.is_empty() {
            press(&world, &mut app, ButtonRole::Cancel);
        }
        std::fs::write(
            &path,
            now.replace("mine now", "first-desc")
                .replace("theirs", "first-desc"),
        )
        .unwrap();
    }
}

#[test]
fn a11_25_the_grammar_item_dialog_and_core_naming_the_item() {
    let (world, mut app) = started();
    open_editor(&world, &mut app, "c-cpp");
    let svc = world.service.clone();
    // Add…: the dialog over the editor; a Columns item whose columns are reversed is refused by core, naming it.
    editor(&mut app).item_selected = Some(0);
    editor(&mut app).item_dialog(None);
    world.settle(&mut app);
    let w = app.win();
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<GrammarItemDialog>())
        .expect("the dialog");
    assert_eq!(d.help_topic(), "dialog-grammar-item");
    assert!(d.names.contains(&"Comment".to_owned()));
    d.fields.element = "Margin".into();
    d.fields.category = Category::Columns;
    d.fields.from_column = "9".into();
    d.fields.to_end = false;
    d.fields.to_column = "3".into();
    assert_eq!(d.on_button(ButtonRole::Ok, &svc), DialogStep::Stay);
    assert!(
        d.error.as_ref().unwrap().starts_with("Grammar item 2"),
        "{:?}",
        d.error
    );
    d.fields.element = "Everything Else".into();
    assert_eq!(d.on_button(ButtonRole::Ok, &svc), DialogStep::Stay);
    assert!(d.error.as_ref().unwrap().contains("reserved"));
    d.fields.element = "Margin".into();
    d.fields.to_column = "12".into();
    d.fields.case_sensitive = false;
    press(&world, &mut app, ButtonRole::Ok);
    let e = editor(&mut app);
    assert_eq!(
        e.format.grammar[1],
        GrammarItem {
            element: hikasa_formats::ElementName::new("Margin").unwrap(),
            kind: ItemKind::Columns {
                from_column: 9,
                to_column: Some(12),
            },
        },
        "after the selected item"
    );
    assert!(
        !e.element_info("Margin").case_sensitive,
        "FF-025: the element's flag"
    );
    // Clone, Move, Delete.
    e.item_selected = Some(1);
    e.clone_item();
    assert_eq!(e.format.grammar[1], e.format.grammar[2]);
    e.move_item(-1);
    assert_eq!(e.item_selected, Some(1));
    e.delete_item();
    e.item_selected = Some(1);
    e.delete_item();
    assert_eq!(
        svc.format_for_edit("c-cpp").unwrap().format.grammar,
        e.format.grammar
    );
    // OK with a bad pattern in the line weights: core's message names it, nothing written.
    e.weight.pattern = "(".into();
    e.weight.regex = true;
    e.add_weight();
    press(&world, &mut app, ButtonRole::Ok);
    let message = editor(&mut app).error.clone().unwrap_or_default();
    assert!(message.starts_with("Line weight "), "{message}");
    assert!(!world.path("config/formats/c-cpp.toml").exists());
}

#[test]
fn a11_25_the_preview_equals_format_preview() {
    let (world, mut app) = started();
    open_editor(&world, &mut app, "c-cpp");
    let svc = world.service.clone();
    let wake: crate::job::Wake = std::sync::Arc::new(|| {});
    let e = editor(&mut app);
    e.preview.sample = "/* x */ int y; // z\n\"s\"\n".into();
    let format = e.result().unwrap();
    // Not before the pause after the last change.
    let at = std::time::Instant::now();
    e.preview.touch(at);
    assert!(!e.preview.tick(&svc, &wake, Some(&format), at));
    assert!(e.preview.tick(
        &svc,
        &wake,
        Some(&format),
        at + crate::formats::preview::PAUSE
    ));
    let began = std::time::Instant::now();
    while e.preview.running() {
        e.preview.tick(
            &svc,
            &wake,
            Some(&format),
            at + crate::formats::preview::PAUSE * 2,
        );
        assert!(began.elapsed() < std::time::Duration::from_secs(30));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let job = svc.job(
        hikasa_core::JobKind::FormatTokens,
        std::sync::Arc::new(hikasa_core::NullProgress),
    );
    let want = svc
        .format_preview(&format, &e.preview.sample, &job)
        .unwrap();
    assert_eq!(e.preview.lines, want);
    assert!(
        e.preview.lines[0]
            .tokens
            .iter()
            .any(|t| t.element == "Comment")
    );
}

/// D-320: each item at the caret (inside the brackets of *Any of*, *None of*) in every regex field the GUI has.
#[test]
fn a11_25_the_regex_helper_in_every_regex_field() {
    let (world, mut app) = started();
    open_editor(&world, &mut app, "c-cpp");
    let mut tabs = crate::text_tabs::ImportanceTab::default();
    let mut find = Fixture::new("a\n", "b\n");
    let mut files =
        crate::text::find_files::FindFilesDialog::new(app.state(), Vec::new(), String::new());
    let item = GrammarItem {
        element: hikasa_formats::ElementName::new("X").unwrap(),
        kind: ItemKind::Basic {
            text: String::new(),
            regex: true,
            match_case: true,
        },
    };
    let format = editor(&mut app).format.clone();
    let mut gi = GrammarItemDialog::new(&format, None, 0, item);
    let e = editor(&mut app);
    let fields: Vec<(&str, &mut String)> = vec![
        ("find panel", &mut find.m().find.query.text),
        ("find in files", &mut files.text),
        ("grammar item text", &mut gi.fields.text),
        ("grammar item from", &mut gi.fields.from),
        ("grammar item to", &mut gi.fields.to),
        ("grammar item heading", &mut gi.fields.heading),
        ("line weight", &mut e.weight.pattern),
        ("unimportant text", &mut tabs.pattern.pattern),
    ];
    for (what, field) in fields {
        for (k, it) in hikasa_formats::regex_helper::ITEMS.iter().enumerate() {
            *field = "ab".into();
            let caret = helper_insert(field, 1..1, k);
            assert_eq!(*field, format!("a{}b", it.insert), "{what} {}", it.caption);
            assert_eq!(caret, 1 + it.caret, "{what} {}", it.caption);
            // A selection is replaced.
            *field = "abc".into();
            helper_insert(field, 1..2, k);
            assert_eq!(*field, format!("a{}c", it.insert), "{what} {}", it.caption);
        }
    }
    let any = hikasa_formats::regex_helper::ITEMS
        .iter()
        .position(|i| i.caption == "Any of")
        .unwrap();
    let mut s = String::new();
    assert_eq!(helper_insert(&mut s, 0..0, any), 1, "inside the brackets");
}

/// W11c review F1: an element setting set back to its default in the editor (a row equal to the default is no row) is
/// saved as such: the file it replaces does not bring the old row back.
#[test]
fn an_element_setting_back_to_its_default_is_saved() {
    let (world, mut app) = started();
    for important in [false, true] {
        open_editor(&world, &mut app, "c-cpp");
        let e = editor(&mut app);
        let mut info = e.element_info("Comment");
        info.important = important;
        e.set_element_info("Comment", info);
        press(&world, &mut app, ButtonRole::Ok);
        let back = world.service.format_for_edit("c-cpp").unwrap().format;
        let comment = back
            .elements
            .iter()
            .find(|(k, _)| k.as_str() == "Comment")
            .map(|(_, v)| v.important);
        assert_eq!(comment.unwrap_or(true), important, "saved {important}");
    }
}

/// W11c review F4: a format renamed in the editor offers the views using it (by its old name) *Reopen*.
#[test]
fn a_renamed_format_offers_reopen() {
    let mut f = Fixture::new("int a;\n", "int b;\n");
    open_editor(&f.world, &mut f.app, "c-cpp");
    editor(&mut f.app).format.name = "C family".into();
    press(&f.world, &mut f.app, ButtonRole::Ok);
    f.settle();
    assert_eq!(
        f.world
            .service
            .format_for_edit("c-cpp")
            .unwrap()
            .format
            .name,
        "C family"
    );
    let reopen = notices(&f.app).into_iter().any(|(_, text, actions)| {
        text == "File format C, C++, C#, ObjC Source changed on disk."
            && actions == vec![crate::notify::NoticeAction::FormatReopen(f.view)]
    });
    assert!(reopen, "{:?}", notices(&f.app));
}

/// W11c review F3: *Load sample…* of a file with a BOM and of a UTF-16 file: the sample is the file's text, and the
/// preview equals `format tokens` of the file.
#[test]
fn a_loaded_sample_previews_as_format_tokens() {
    let (world, mut app) = started();
    open_editor(&world, &mut app, "c-cpp");
    let svc = world.service.clone();
    let text = "int x; // c\n/* é */\n";
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let bom = [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat();
    for (name, bytes) in [("bom.c", bom), ("wide.c", utf16)] {
        let path = world.write(&format!("work/{name}"), "");
        std::fs::write(&path, &bytes).unwrap();
        editor(&mut app).load_sample(&svc, path.clone());
        let began = std::time::Instant::now();
        while editor(&mut app).preview.running() || editor(&mut app).preview.sample != text {
            world.settle(&mut app);
            assert!(
                began.elapsed() < std::time::Duration::from_secs(60),
                "{name}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let format = editor(&mut app).result().unwrap();
        let job = |svc: &hikasa_session::Service| {
            svc.job(
                hikasa_core::JobKind::FormatTokens,
                std::sync::Arc::new(hikasa_core::NullProgress),
            )
        };
        let preview = svc.format_preview(&format, text, &job(&svc)).unwrap();
        let tokens = svc
            .format_tokens(
                &hikasa_session::api::FormatTokensRequest {
                    file: path.into_os_string(),
                    format: Some("c-cpp".into()),
                    encoding: None,
                    lines: None,
                },
                &job(&svc),
            )
            .unwrap();
        assert_eq!(preview, tokens.lines, "{name}");
    }
}

/// The editor's fields: `tab` (or `\t`) is a tab delimiter, a line weight takes its typed priority, *Disable editing*
/// removes the save command line and unticked gives an empty one.
#[test]
fn the_editor_fields_read_as_typed() {
    use hikasa_formats::Conversion;
    let (world, mut app) = started();
    open_editor(&world, &mut app, "csv");
    let e = editor(&mut app);
    for (typed, want) in [("tab", '\t'), ("\\t", '\t'), (";", ';')] {
        e.typed.delimiter = typed.into();
        assert_eq!(
            e.result().unwrap().table.unwrap().delimiter,
            want,
            "{typed}"
        );
    }
    e.typed.delimiter = "ab".into();
    assert!(e.result().unwrap_err().contains("one character"));
    press(&world, &mut app, ButtonRole::Cancel);
    open_editor(&world, &mut app, "c-cpp");
    let e = editor(&mut app);
    e.weight.pattern = "TODO".into();
    e.typed.priority = "3".into();
    e.add_weight();
    assert_eq!(e.format.line_weights.last().unwrap().priority, 3);
    e.set_conversion(ConversionChoice::External);
    e.external_mut().unwrap().save = Some("back %s %t".into());
    e.set_disable_editing(true);
    let save = |e: &FormatEditor| match &e.format.conversion {
        Conversion::External(x) => x.save.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(save(e), None, "disabled: no save command line");
    e.set_disable_editing(false);
    assert_eq!(save(e), Some(String::new()));
}

/// The manager: *Restore Built-in* only for a built-in with an override file; *Create* with no id keeps the prompt
/// with a message and writes nothing.
#[test]
fn restore_only_for_an_override_and_new_needs_an_id() {
    let (world, mut app) = started();
    world
        .service
        .format_new("mine", None, Some("Mine"), Some("*.mine"), None)
        .unwrap();
    world
        .service
        .format_edit("java", &[("description".into(), "\"mine\"".into())], None)
        .unwrap();
    world.service.reload_settings();
    open_manager(&world, &mut app);
    for (id, restore) in [("mine", false), ("c-cpp", false), ("java", true)] {
        select(&mut app, id);
        assert_eq!(manager(&mut app).can_restore(), restore, "{id}");
    }
    let before = formats_dir(&world);
    let m = manager(&mut app);
    m.start_new(FormatKind::Text);
    m.prompt.as_mut().unwrap().id = "  ".into();
    m.prompt.as_mut().unwrap().name = "Named".into();
    m.create();
    assert_eq!(m.error.as_deref(), Some("Type an id for the new format."));
    assert!(m.prompt.is_some(), "the prompt stays");
    world.settle(&mut app);
    assert_eq!(formats_dir(&world), before, "nothing written");
}
