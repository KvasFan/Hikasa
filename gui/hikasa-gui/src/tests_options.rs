//! M2 W4c acceptance (§6.4 A4.17–A4.21) through the toolkit-free models (§1.4.16): the Options dialog, the commands
//! page, the theme editor, the wizards and `ImportPackage`. No window opens.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use hikasa_session::FixedClock;
use hikasa_session::ipc::{IpcHandler, IpcServer, Request, Response};
use hikasa_session::package::{Category, ExportRequest, ImportChoices, PlanAction};
use hikasa_session::settings::commands::DisplayFilterMode;
use hikasa_session::settings::theme::{SchemeBase, SchemeGroup};
use hikasa_session::settings::{
    Colour, FileId, FormValue, Scheme, Shortcut, ThemeMode, ValueKind, enum_caption, key_infos,
    parse_key,
};

use crate::app::{AppCore, Effect};
use crate::commands::{CommandId, Scope};
use crate::dialog::{ButtonRole, Prompt};
use crate::ipc::GuiHandler;
use crate::menus::{MenuContext, menu_bar, render_text};
use crate::notify::NoticeAction;
use crate::options::OptionsDialog;
use crate::options::commands_page::ViewFlag;
use crate::options::pages::{Body, PAGES, page_of};
use crate::options::theme_page::FontSlot;
use crate::tests::{TestApp, World, text_pair};
use crate::watch::{WatchEvent, Watcher};
use crate::window::{CommandArg, Invocation, WindowId};
use crate::wizards::{ExportWizard, ImportWizard, RestoreWizard, summary_lines};

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("HIKASA_BLESS").is_some() {
        std::fs::write(&path, actual).expect("bless");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (bless with HIKASA_BLESS=1)", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "{name}");
}

fn open(app: &mut TestApp) -> WindowId {
    let w = app.root();
    app.0.execute(w, CommandId::ProgramOptions);
    w
}

fn dlg(app: &mut AppCore, w: WindowId) -> &mut OptionsDialog {
    app.window_mut(w)
        .expect("window")
        .dialogs
        .find_mut::<OptionsDialog>()
        .expect("the Options dialog")
}

fn press(world: &World, app: &mut TestApp, w: WindowId, role: ButtonRole) -> Vec<Effect> {
    let mut effects = app.0.dialog_button(w, role);
    effects.extend(world.settle(&mut app.0));
    effects
}

fn enabled(app: &mut AppCore, w: WindowId, role: ButtonRole) -> bool {
    use crate::dialog::DialogModel;
    dlg(app, w)
        .buttons()
        .iter()
        .any(|b| b.role == role && b.enabled)
}

fn files(world: &World) -> BTreeMap<String, String> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.insert(rel.display().to_string().replace('\\', "/"), text);
            }
        }
    }
    let mut out = BTreeMap::new();
    let base = world.path("config");
    walk(&base, &base, &mut out);
    out.remove("state.toml");
    out
}

fn top_prompt(app: &AppCore, w: WindowId) -> Option<Prompt> {
    app.window(w)?
        .dialogs
        .top()?
        .as_any()
        .downcast_ref::<Prompt>()
        .cloned()
}

fn sc(text: &str) -> Shortcut {
    Shortcut::parse(text).expect("a shortcut")
}

// ---- A4.17 the Options dialog ----------------------------------------------------------------------------------

#[test]
fn every_p0_key_is_on_exactly_one_page_and_no_other_key_is_shown() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    let infos = key_infos(&FileId::Options);
    let shown: Vec<&str> = PAGES
        .iter()
        .flat_map(|p| p.fields)
        .map(|f| f.path)
        .collect();
    for info in &infos {
        let path = info.key.strip_prefix("options.").expect("the prefix");
        let on_pages = shown.iter().filter(|p| **p == path).count();
        match info.priority {
            Some("P0") => {
                assert_eq!(on_pages, 1, "{path}");
                assert!(d.field(path).is_some(), "{path}: on a page, but no field");
            }
            _ => assert_eq!(on_pages, 0, "{path} is not P0"),
        }
    }
    assert_eq!(
        shown.len(),
        infos.iter().filter(|i| i.priority == Some("P0")).count(),
        "a page names a key the schema does not have"
    );
}

#[test]
fn every_page_of_the_contract_exists_and_lists_are_on_theirs() {
    for id in [
        "startup",
        "tabs",
        "appearance",
        "appearance.folder",
        "appearance.file",
        "appearance.picture",
        "appearance.fonts",
        "text_editing",
        "next_difference",
        "backups",
        "file_operations",
        "archive_types",
        "commands",
        "open_with",
        "tweaks.editor_display",
        "tweaks.file_operations",
        "tweaks.interface",
        "tweaks.log_panel",
        "tweaks.metrics",
        "tweaks.name_filter_presets",
        "tweaks.scripts",
    ] {
        assert!(page_of(id).is_some(), "{id}");
    }
    assert_eq!(page_of("commands").map(|p| p.body), Some(Body::Commands));
    for page in PAGES {
        if let Some(parent) = page.parent {
            assert!(page_of(parent).is_some(), "{}", page.id);
        }
    }
}

#[test]
fn every_enum_value_on_a_page_has_a_caption() {
    let infos = key_infos(&FileId::Options);
    for def in PAGES.iter().flat_map(|p| p.fields) {
        let key = format!("options.{}", def.path);
        let info = infos.iter().find(|i| i.key == key).expect("a key");
        if let ValueKind::Enum(values) = info.kind {
            for value in values {
                assert!(enum_caption(value).is_some(), "{key}: {value}");
            }
        }
    }
}

#[test]
fn widgets_clamp_at_both_range_bounds() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    let path = "tweaks.log_panel.display_up_to";
    d.set_field(path, FormValue::Int(1));
    assert_eq!(d.options.tweaks.log_panel.display_up_to, 10);
    assert_eq!(
        d.field(path).map(|f| f.value.clone()),
        Some(FormValue::Int(10))
    );
    d.set_field(path, FormValue::Int(i64::MAX));
    assert_eq!(d.options.tweaks.log_panel.display_up_to, 100_000);
    d.set_font_size(FontSlot::Editor, 1000.0);
    assert_eq!(d.theme.fonts.editor.size, 72.0);
    d.set_font_size(FontSlot::Editor, 1.0);
    assert_eq!(d.theme.fonts.editor.size, 6.0);
    assert!(d.problems().is_empty());
}

#[test]
fn the_search_covers_titles_labels_key_paths_and_opt_ids() {
    let world = World::with_files(A1_2);
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    let ids = |d: &OptionsDialog| -> Vec<&'static str> {
        d.visible_pages().iter().map(|p| p.id).collect()
    };
    assert_eq!(ids(d).len(), PAGES.len());
    d.search = "next difference".into(); // a title (and labels)
    assert!(ids(d).contains(&"next_difference"));
    d.search = "BACKSPACE".into(); // a label, in any case
    assert_eq!(ids(d), ["text_editing"]);
    d.search = "tweaks.metrics.hint_show_ms".into(); // a key path, with its parent
    assert_eq!(ids(d), ["tweaks", "tweaks.metrics"]);
    d.search = "opt-012".into(); // an OPT id of a list page
    assert_eq!(ids(d), ["open_with"]);
    d.search = "OPT-004".into(); // an OPT id of the fields
    assert_eq!(ids(d), ["tabs"]);
    d.search = "orphan".into(); // an element of a scheme
    assert!(ids(d).contains(&"appearance.folder"));
    d.search = "no such thing".into();
    assert!(ids(d).is_empty());
}

#[test]
fn apply_with_no_change_writes_nothing() {
    let world = World::with_files(&[("options.toml", "schema_version = 1\n# a comment\n")]);
    let mut app = world.app(None);
    let before = files(&world);
    let w = open(&mut app);
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(files(&world), before);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(1));
    // OK with nothing to do closes at once.
    press(&world, &mut app, w, ButtonRole::Ok);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(0));
    assert_eq!(files(&world), before);
}

/// The fixture of A1.2: unknown top-level sections, unknown keys inside known tables and inside `[[app]]` and
/// `[[preset]]` entries, and keys of later priorities.
const A1_2: &[(&str, &str)] = &[
    (
        "options.toml",
        "schema_version = 1\nfuture = 1\n[tabs]\nfuture_key = 3\nshow_taskbar_previews = true\n\
         [tweaks.check_for_updates]\nevery_days = 30\n[colours.deep]\nred = \"#ff0000\"\n",
    ),
    (
        "filters.toml",
        "schema_version = 1\n\n[[preset]]\nname = \"A\"\nmask = \"*.a\"\nicon = \"star\"\n",
    ),
    (
        "openwith.toml",
        "schema_version = 1\n\n[[app]]\ndescription = \"Ed\"\ncommand = \"ed %f\"\nnew_flag = true\n",
    ),
];

#[test]
fn apply_of_one_change_equals_settings_set_and_keeps_hidden_keys() {
    let world = World::with_files(A1_2);
    let twin = World::with_files(A1_2);
    let mut app = world.app(None);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    press(&world, &mut app, w, ButtonRole::Apply);
    twin.service
        .settings_set(
            &parse_key("options.tabs.hide_tab_bar_if_single_tab").expect("a key"),
            "true",
            false,
        )
        .expect("set");
    let (got, want) = (files(&world), files(&twin));
    assert_eq!(got, want);
    assert_eq!(got.len(), 3, "the fixture's files only: {:?}", got.keys());
    for (name, text) in A1_2.iter().skip(1) {
        assert_eq!(got[*name], *text, "only options.toml changes");
    }
    let text = &got["options.toml"];
    for kept in [
        "future = 1",
        "red = \"#ff0000\"",
        "future_key = 3",
        "show_taskbar_previews = true",
        "every_days = 30",
    ] {
        assert!(text.contains(kept), "{text}");
    }
    // The working copy follows the sparse file: nothing more to write.
    assert!(dlg(&mut app, w).changes().options.is_none());
}

#[test]
fn an_invalid_value_disables_apply() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    assert!(enabled(&mut app, w, ButtonRole::Apply));
    let d = dlg(&mut app, w);
    d.set_archive_mask(hikasa_session::settings::archives::ArchiveType::Zip, "[");
    assert!(
        d.problems()[0].contains("archives: types.zip"),
        "{:?}",
        d.problems()
    );
    assert!(!enabled(&mut app, w, ButtonRole::Apply));
    assert!(!enabled(&mut app, w, ButtonRole::Ok));
    let before = files(&world);
    press(&world, &mut app, w, ButtonRole::Ok);
    assert_eq!(files(&world), before);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(1));
    dlg(&mut app, w).set_archive_mask(
        hikasa_session::settings::archives::ArchiveType::Zip,
        "*.zip",
    );
    assert!(enabled(&mut app, w, ButtonRole::Apply));
}

fn conflicted(world: &World, app: &mut TestApp) -> WindowId {
    let w = open(app);
    dlg(app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    // A concurrent CLI edit.
    world
        .service
        .settings_set(
            &parse_key("options.tabs.open_new_tab_next_to_active").expect("a key"),
            "false",
            false,
        )
        .expect("set");
    press(world, app, w, ButtonRole::Ok);
    let prompt = top_prompt(app, w).expect("the conflict prompt");
    assert!(
        prompt.message.contains("options.toml"),
        "{}",
        prompt.message
    );
    assert_eq!(prompt.buttons.len(), 3);
    w
}

#[test]
fn a_conflict_can_overwrite() {
    let world = World::new();
    let mut app = world.app(None);
    let w = conflicted(&world, &mut app);
    press(&world, &mut app, w, ButtonRole::Other(1));
    let text = std::fs::read_to_string(world.path("config/options.toml")).expect("options");
    assert!(text.contains("hide_tab_bar_if_single_tab = true"), "{text}");
    assert!(!text.contains("open_new_tab_next_to_active"), "{text}");
    // OK closes once the file went through.
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(0));
}

#[test]
fn a_conflict_can_reload_the_file() {
    let world = World::new();
    let mut app = world.app(None);
    let w = conflicted(&world, &mut app);
    press(&world, &mut app, w, ButtonRole::Other(2));
    let d = dlg(&mut app, w);
    assert!(!d.options.tabs.hide_tab_bar_if_single_tab);
    assert!(!d.options.tabs.open_new_tab_next_to_active);
    assert_eq!(
        d.field("tabs.open_new_tab_next_to_active")
            .map(|f| f.value.clone()),
        Some(FormValue::Bool(false))
    );
    assert!(d.changes().options.is_none());
    let text = std::fs::read_to_string(world.path("config/options.toml")).expect("options");
    assert!(!text.contains("hide_tab_bar"), "{text}");
}

#[test]
fn a_conflict_can_be_cancelled_and_ok_stays() {
    let world = World::new();
    let mut app = world.app(None);
    let w = conflicted(&world, &mut app);
    press(&world, &mut app, w, ButtonRole::Cancel);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(1), "OK stayed");
    let d = dlg(&mut app, w);
    assert!(d.options.tabs.hide_tab_bar_if_single_tab, "the edit stays");
    // The next Apply meets the same conflict: the file's stamp was not refreshed.
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(top_prompt(&app, w).is_some());
}

#[test]
fn cancel_leaves_the_files_untouched() {
    let world = World::new();
    let mut app = world.app(None);
    let before = files(&world);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    press(&world, &mut app, w, ButtonRole::Cancel);
    assert_eq!(files(&world), before);
    assert_eq!(app.window(w).map(|w| w.dialogs.len()), Some(0));
    assert!(!app.options().tabs.hide_tab_bar_if_single_tab);
}

#[test]
fn the_window_applies_the_new_values_after_apply() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    d.set_field("tweaks.metrics.hint_show_ms", FormValue::Int(77));
    d.set_field("tweaks.log_panel.warnings", FormValue::Bool(false));
    d.set_field("tweaks.log_panel.display_up_to", FormValue::Int(123));
    d.set_field("text_editing.number_of_context_lines", FormValue::Int(9));
    press(&world, &mut app, w, ButtonRole::Ok);
    assert!(!app.tab_bar_visible(w), "tabs");
    assert_eq!(app.options().tweaks.metrics.hint_show_ms, 77);
    assert_eq!(
        app.hints(),
        crate::commands::HintTiming::from_options(app.options())
    );
    assert!(!app.log().filter().warnings, "log filter");
    assert_eq!(app.log().filter().capacity, 123);
    // Own writes reach the service too (D-119).
    assert_eq!(world.service.options().context_lines, 9);
}

#[test]
fn the_dialog_opens_on_the_page_of_the_argument_or_the_last_one() {
    let world = World::new();
    let mut app = world.app(None);
    let w = app.root();
    app.0.execute(
        w,
        Invocation::with(CommandId::ProgramOptions, CommandArg::Page("tabs")),
    );
    assert_eq!(dlg(&mut app, w).page, "tabs");
    dlg(&mut app, w).show_page("backups");
    press(&world, &mut app, w, ButtonRole::Cancel);
    app.flush(std::time::Duration::from_secs(5));
    open(&mut app);
    assert_eq!(dlg(&mut app, w).page, "backups");
    press(&world, &mut app, w, ButtonRole::Cancel);
    let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state");
    assert!(state.contains("page = \"backups\""), "{state}");
    // A new process remembers it.
    let mut again = world.app(None);
    let w = open(&mut again);
    assert_eq!(dlg(&mut again, w).page, "backups");
}

#[test]
fn a_second_options_dialog_raises_the_first() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let second = {
        // The menus are off while a dialog is open: the second window opens first.
        press(&world, &mut app, w, ButtonRole::Cancel);
        app.0.execute(w, CommandId::WindowNew);
        app.windows()[1].id
    };
    open(&mut app);
    let effects = app.0.execute(second, CommandId::ProgramOptions);
    assert_eq!(effects, vec![Effect::Raise(w)]);
    assert!(app.window(second).is_some_and(|x| x.dialogs.is_empty()));
}

#[test]
fn a_changed_file_with_unapplied_edits_offers_reload_or_keep() {
    let world = World::new();
    let mut app = world.app(None);
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    let outside = |value: &str| {
        world
            .service
            .settings_set(
                &parse_key("options.tweaks.metrics.max_mru_items").expect("a key"),
                value,
                false,
            )
            .expect("set");
    };
    outside("5");
    let events: Vec<WatchEvent> = watcher.poll_once().expect("alive");
    app.on_watch(events);
    world.settle(&mut app);
    let notice = app
        .window_mut(w)
        .and_then(|x| x.notices.visible(0.0).first().map(|n| (*n).clone()))
        .expect("a notice");
    assert_eq!(
        notice.actions,
        vec![NoticeAction::ReloadSettings, NoticeAction::KeepMine]
    );
    // Keep my edits: the next Apply meets the conflict.
    app.notice_action(w, &notice.key, NoticeAction::KeepMine);
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(top_prompt(&app, w).is_some(), "Conflict");
    press(&world, &mut app, w, ButtonRole::Cancel);
    // Reload: the edits go and the dialog shows the file.
    outside("6");
    let events = watcher.poll_once().expect("alive");
    app.on_watch(events);
    world.settle(&mut app);
    let key = notice.key;
    app.notice_action(w, &key, NoticeAction::ReloadSettings);
    world.settle(&mut app);
    let d = dlg(&mut app, w);
    assert!(!d.options.tabs.hide_tab_bar_if_single_tab);
    assert_eq!(d.options.tweaks.metrics.max_mru_items, 6);
}

#[test]
fn a_file_without_edits_follows_the_disk_silently() {
    let world = World::new();
    let mut app = world.app(None);
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let w = open(&mut app);
    world
        .service
        .settings_set(
            &parse_key("options.tweaks.metrics.max_mru_items").expect("a key"),
            "5",
            false,
        )
        .expect("set");
    let events = watcher.poll_once().expect("alive");
    app.on_watch(events);
    world.settle(&mut app);
    assert_eq!(dlg(&mut app, w).options.tweaks.metrics.max_mru_items, 5);
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(top_prompt(&app, w).is_none());
}

#[test]
fn the_own_apply_raises_no_notice() {
    let world = World::new();
    let mut app = world.app(None);
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    d.commands.view_mut("home").large_buttons = true;
    "mine".clone_into(&mut d.theme_page.new_name);
    d.new_scheme();
    d.set_element_colour(SchemeGroup::File, "gutter", RED);
    press(&world, &mut app, w, ButtonRole::Ok);
    assert!(world.path("config/themes/mine.toml").exists());
    assert_eq!(watcher.poll_once().expect("alive"), Vec::new());
}

#[test]
fn the_open_options_notice_action_opens_the_dialog() {
    let world = World::new();
    let mut app = world.app(None);
    let w = app.root();
    app.window_mut(w).expect("window").notices.push(
        crate::notify::Notice::new("k", crate::notify::NoticeKind::Warning, "text")
            .with_actions(vec![NoticeAction::OpenOptions]),
        0.0,
    );
    app.notice_action(w, "k", NoticeAction::OpenOptions);
    assert!(
        app.window(w)
            .is_some_and(|x| x.dialogs.holds::<OptionsDialog>())
    );
}

#[test]
fn open_with_and_presets_edit_their_lists() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.open_with_new();
    let edit = d.open_with.editing.as_mut().expect("editing");
    edit.app.description = "Editor".into();
    edit.app.command = "ed \"%f".into();
    d.open_with_commit();
    assert!(
        d.open_with
            .editing
            .as_ref()
            .is_some_and(|e| e.error.is_some())
    );
    assert!(d.openwith.app.is_empty(), "a bad item stays in the editor");
    "ed %f".clone_into(&mut d.open_with.editing.as_mut().expect("editing").app.command);
    d.open_with_commit();
    assert_eq!(d.openwith.app.len(), 1);
    d.open_with.selected = Some(0);
    d.open_with_clone();
    d.open_with_commit();
    assert_eq!(d.openwith.app[1].description, "Editor (copy)");
    d.open_with.selected = Some(0);
    d.open_with_edit();
    "Vi".clone_into(
        &mut d
            .open_with
            .editing
            .as_mut()
            .expect("editing")
            .app
            .description,
    );
    d.open_with_commit();
    assert_eq!(d.openwith.app[0].description, "Vi");
    d.open_with.selected = Some(1);
    d.open_with_delete();
    assert_eq!(d.openwith.app.len(), 1);
    let presets = d.filters.preset.len();
    d.preset_add();
    assert_eq!(d.filters.preset.len(), presets + 1);
    d.filters.preset[presets].name = d.filters.preset[0].name.clone();
    d.edited();
    assert!(!d.problems().is_empty(), "a duplicate name");
    d.preset_delete(presets);
    assert!(d.problems().is_empty());
    press(&world, &mut app, w, ButtonRole::Ok);
    let text = std::fs::read_to_string(world.path("config/openwith.toml")).expect("written");
    assert!(text.contains("description = \"Vi\""), "{text}");
    assert!(!world.path("config/filters.toml").exists());
}

// ---- A4.18 Toolbars, Shortcuts, Menus --------------------------------------------------------------------------

fn select(d: &mut OptionsDialog, view: &'static str, id: &str) {
    d.select_view(view);
    d.select_command(id);
}

fn row<'a>(d: &'a OptionsDialog, id: &str) -> &'a crate::options::commands_page::CommandRow {
    d.commands_page
        .rows
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("{id}"))
}

const PLANNED: &str = "schema_version = 1\n[view.home.command.FileFormatManager]\nmenu = true\n\
shortcuts = [\"Ctrl+F2\"]\n";

#[test]
fn the_commands_page_writes_commands_toml_and_the_window_follows() {
    let world = World::with_files(&[("commands.toml", PLANNED)]);
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.show_page("commands");
    // Add, Remove, Default.
    select(d, "home", "About");
    d.begin_capture();
    assert!(d.capturing());
    d.capture(&sc("Ctrl+F12"));
    assert!(!d.capturing());
    assert_eq!(row(d, "About").shortcuts, vec![sc("Ctrl+F12")]);
    select(d, "home", "TabNew");
    d.commands_page.shortcut = Some(0);
    d.remove_shortcut();
    assert!(row(d, "TabNew").shortcuts.is_empty());
    select(d, "home", "WindowNew");
    d.begin_capture();
    d.capture(&sc("Alt+W"));
    d.default_shortcuts();
    assert!(row(d, "WindowNew").shortcuts.is_empty());
    // Esc gives the capture up.
    d.begin_capture();
    d.capture(&sc("Esc"));
    assert!(row(d, "WindowNew").shortcuts.is_empty());
    // Flags.
    d.set_command_menu("Exit", false);
    assert_eq!(row(d, "Exit").menu, Some(false));
    d.set_filter_mode(DisplayFilterMode::Favorites);
    d.set_view_flag(ViewFlag::LargeButtons, true);
    d.select_view("text-compare");
    d.set_command_toolbar("Reload", false);
    assert_eq!(row(d, "Reload").toolbar, Some(false));
    d.set_view_flag(ViewFlag::ShowAllInMenu, true);
    d.set_view_flag(ViewFlag::ShowAllOnToolbar, true);
    // The search: id, caption, description, shortcut.
    d.set_command_search("ctrl+t");
    assert!(d.commands_page.rows.iter().any(|r| r.id == "TabNew"));
    d.set_command_search("");
    press(&world, &mut app, w, ButtonRole::Ok);
    let text = std::fs::read_to_string(world.path("config/commands.toml")).expect("written");
    golden("commands-after-options.toml", &text);
    // The planned override is untouched.
    assert!(
        text.contains("[view.home.command.FileFormatManager]"),
        "{text}"
    );
    // The window follows: dispatch and menus.
    assert_eq!(
        app.dispatch_key(w, &sc("Ctrl+F12"), crate::commands::FocusArea::Root),
        Some(CommandId::About)
    );
    assert_eq!(
        app.dispatch_key(w, &sc("Ctrl+T"), crate::commands::FocusArea::Root),
        None
    );
    let checked = |_: CommandId| None;
    let mc = MenuContext {
        tables: app.tables(),
        scope: Scope::Home,
        ctx: app.0.ctx(w),
        checked: &checked,
        workspaces: &[],
    };
    golden("menus-home-options.txt", &render_text(&menu_bar(&mc)));
}

#[test]
fn removing_all_shortcuts_writes_an_empty_list_and_planned_commands_are_hidden() {
    let world = World::with_files(&[("commands.toml", PLANNED)]);
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    assert!(
        !d.commands_page
            .rows
            .iter()
            .any(|r| r.id == "FileFormatManager")
    );
    select(d, "home", "TabNew");
    d.commands_page.shortcut = Some(0);
    d.remove_shortcut();
    d.reset_commands();
    assert_eq!(
        d.changes().commands,
        None,
        "only the planned override is left"
    );
    d.commands_page.selected = Some("TabNew".into());
    d.commands_page.shortcut = Some(0);
    d.remove_shortcut();
    assert_eq!(
        d.commands.view["home"].command["TabNew"].shortcuts,
        Some(Vec::new())
    );
    press(&world, &mut app, w, ButtonRole::Ok);
    let text = std::fs::read_to_string(world.path("config/commands.toml")).expect("written");
    assert!(text.contains("shortcuts = []"), "{text}");
}

#[test]
fn conflicts_are_live_within_a_view_never_across_views_and_do_not_block() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    select(d, "home", "About");
    d.begin_capture();
    d.capture(&sc("Ctrl+T"));
    assert_eq!(row(d, "About").conflicts.len(), 1, "{:?}", row(d, "About"));
    assert!(row(d, "About").conflicts[0].contains("Ctrl+T"));
    assert_eq!(row(d, "TabNew").conflicts.len(), 1, "shown on both rows");
    d.select_view("folder-compare");
    assert!(row(d, "TabNew").conflicts.is_empty(), "none across views");
    assert!(
        enabled(&mut app, w, ButtonRole::Apply),
        "never blocks Apply"
    );
}

#[test]
fn a_key_goes_to_the_capture_while_it_waits() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    select(d, "home", "About");
    d.begin_capture();
    assert!(app.window(w).is_some_and(|x| x.dialogs.wants_keys()));
    app.0.dialog_key(w, &sc("F1"));
    assert_eq!(app.0.help(w), None, "F1 is captured, not help");
    assert_eq!(row(dlg(&mut app, w), "About").shortcuts, vec![sc("F1")]);
    // F1 after that is the dialog's help again.
    app.0.dialog_key(w, &sc("F1"));
    assert_eq!(app.0.help(w), Some("dialog-options"));
}

#[test]
fn toolbar_options_opens_the_commands_page_of_the_view() {
    let world = World::new();
    let (l, r) = text_pair(&world);
    let mut app = world.app(None);
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let w = app.root();
    let view = app.0.scope(w).view_name().expect("a view");
    app.open_commands_page(w, view);
    let d = dlg(&mut app, w);
    assert_eq!(d.page, "commands");
    assert_eq!(d.commands_page.view, "text-compare");
    assert!(d.commands_page.rows.iter().any(|r| r.id == "Reload"));
}

// ---- A4.19 the theme editor ------------------------------------------------------------------------------------

const RED: Colour = Colour::hex(0xff0000);
const BLUE: Colour = Colour::hex(0x0000ff);

fn light_only(world: &World) {
    world
        .service
        .settings_set(
            &parse_key("options.appearance.theme_mode").expect("a key"),
            "light",
            false,
        )
        .expect("set");
}

#[test]
fn a_built_in_round_trips_sparse_and_reset_removes_the_element() {
    let world = World::new();
    light_only(&world);
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.set_element_colour(SchemeGroup::Folder, "orphan", RED);
    d.set_element_colour(SchemeGroup::File, "gutter", BLUE);
    d.set_element_colour(SchemeGroup::Picture, "similar", BLUE);
    d.set_element_flag(SchemeGroup::File, "use_stripes", true);
    d.set_font_size(FontSlot::Editor, 15.0);
    d.set_font_family(FontSlot::Hex, "Mono");
    d.set_folder_system_font(false);
    let (edited, _) = d.working_scheme(false);
    press(&world, &mut app, w, ButtonRole::Apply);
    // settings_snapshot → resolve equals the edited scheme.
    let snapshot = world.service.settings_snapshot().expect("snapshot");
    assert_eq!(snapshot.scheme(ThemeMode::Light, false), edited);
    assert_eq!(*app.scheme(), edited, "the window keeps it");
    let text = std::fs::read_to_string(world.path("config/themes/light.toml")).expect("written");
    golden("scheme-light-sparse.toml", &text);
    let theme = std::fs::read_to_string(world.path("config/theme.toml")).expect("written");
    golden("theme-fonts.toml", &theme);
    // Reset: the element leaves the file.
    let d = dlg(&mut app, w);
    assert!(d.element_set(SchemeGroup::Folder, "orphan"));
    d.reset_element(SchemeGroup::Folder, "orphan");
    assert!(!d.element_set(SchemeGroup::Folder, "orphan"));
    press(&world, &mut app, w, ButtonRole::Apply);
    let text = std::fs::read_to_string(world.path("config/themes/light.toml")).expect("written");
    assert!(!text.contains("orphan"), "{text}");
    // Reset all on a built-in deletes its override file.
    dlg(&mut app, w).reset_all();
    press(&world, &mut app, w, ButtonRole::Ok);
    assert!(!world.path("config/themes/light.toml").exists());
}

#[test]
fn a_value_equal_to_the_base_is_not_stored() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    let base = Scheme::builtin(SchemeBase::Light).folder.orphan;
    d.set_element_colour(SchemeGroup::Folder, "orphan", RED);
    d.set_element_colour(SchemeGroup::Folder, "orphan", base);
    assert!(!d.element_set(SchemeGroup::Folder, "orphan"));
    assert!(d.changes().schemes.is_empty());
}

#[test]
fn new_clone_and_delete_schemes() {
    let world = World::new();
    light_only(&world);
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    // Bad and taken names.
    "a b".clone_into(&mut d.theme_page.new_name);
    d.new_scheme();
    assert!(d.theme_page.error.is_some());
    "LIGHT".clone_into(&mut d.theme_page.new_name);
    d.new_scheme();
    assert!(d.theme_page.error.is_some());
    // New on the dark base.
    "mine".clone_into(&mut d.theme_page.new_name);
    d.theme_page.new_base = SchemeBase::Dark;
    d.new_scheme();
    assert_eq!(d.theme_page.scheme, "mine");
    d.set_element_colour(SchemeGroup::Folder, "orphan", RED);
    d.set_light_scheme("mine");
    let (edited, base) = d.working_scheme(false);
    assert_eq!(base, SchemeBase::Dark);
    assert_eq!(edited.folder.orphan, RED);
    // Clone.
    "copy".clone_into(&mut d.theme_page.new_name);
    d.clone_scheme();
    assert_eq!(d.theme_page.scheme, "copy");
    assert_eq!(d.theme_page.resolved.folder.orphan, RED);
    assert_eq!(d.schemes["copy"].base, Some(SchemeBase::Dark));
    press(&world, &mut app, w, ButtonRole::Apply);
    let text = std::fs::read_to_string(world.path("config/themes/mine.toml")).expect("written");
    golden("scheme-new-sparse.toml", &text);
    assert!(world.path("config/themes/copy.toml").exists());
    let snapshot = world.service.settings_snapshot().expect("snapshot");
    assert_eq!(snapshot.scheme(ThemeMode::Light, false), edited);
    assert_eq!(app.scheme().name, "mine");
    // Delete: a built-in refuses, a scheme in use falls back.
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.delete_scheme();
    assert!(d.theme_page.error.is_some());
    d.select_scheme("mine");
    d.delete_scheme();
    assert_eq!(d.theme.light_scheme, "light");
    press(&world, &mut app, w, ButtonRole::Ok);
    assert!(!world.path("config/themes/mine.toml").exists());
    assert_eq!(app.scheme().name, "light");
}

#[test]
fn each_edit_previews_once_and_cancel_restores() {
    let world = World::new();
    light_only(&world);
    let mut app = world.app(None);
    let saved = app.scheme().clone();
    let w = open(&mut app);
    let count = |effects: &[Effect]| effects.iter().filter(|e| **e == Effect::ApplyTheme).count();
    // No edit: nothing, however often the frame reports.
    assert_eq!(count(&app.0.dialog_edited(w)), 0);
    dlg(&mut app, w).select_scheme("light");
    dlg(&mut app, w).set_element_colour(SchemeGroup::Folder, "orphan", RED);
    assert_eq!(count(&app.0.dialog_edited(w)), 1, "one step");
    assert_eq!(app.scheme().folder.orphan, RED, "the working scheme");
    assert_eq!(count(&app.0.dialog_edited(w)), 0, "once");
    dlg(&mut app, w).set_element_colour(SchemeGroup::Folder, "orphan", BLUE);
    assert_eq!(count(&app.0.dialog_edited(w)), 1);
    assert_eq!(app.scheme().folder.orphan, BLUE);
    // The mode is an option: the preview follows it too.
    dlg(&mut app, w).set_field("appearance.theme_mode", FormValue::Text("dark".into()));
    assert_eq!(count(&app.0.dialog_edited(w)), 1);
    assert_eq!(app.scheme().name, "dark");
    let effects = app.0.dialog_button(w, ButtonRole::Cancel);
    assert_eq!(count(&effects), 1, "Cancel restores");
    assert_eq!(*app.scheme(), saved);
}

#[test]
fn the_font_preview_shows_through_the_theme_accessor() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    dlg(&mut app, w).set_font_size(FontSlot::Folder, 20.0);
    assert_eq!(
        app.0
            .dialog_edited(w)
            .iter()
            .filter(|e| **e == Effect::ApplyTheme)
            .count(),
        1
    );
    assert_eq!(app.theme().fonts.folder.size, 20.0);
    app.0.dialog_button(w, ButtonRole::Cancel);
    assert_eq!(app.theme().fonts.folder.size, 13.0);
}

#[test]
fn closing_the_window_ends_the_preview() {
    let world = World::new();
    light_only(&world);
    let mut app = world.app(None);
    let saved = app.scheme().clone();
    let w = open(&mut app);
    dlg(&mut app, w).select_scheme("light");
    dlg(&mut app, w).set_element_colour(SchemeGroup::Folder, "orphan", RED);
    app.0.dialog_edited(w);
    // The edit is unapplied: the OS close asks first, and the preview ends with the answer.
    app.close_window(w);
    assert_ne!(*app.scheme(), saved, "still previewing");
    let effects = app.0.dialog_button(w, ButtonRole::Other(1));
    assert!(effects.contains(&Effect::ApplyTheme));
    assert_eq!(*app.scheme(), saved);
}

#[test]
fn contrast_below_the_limit_warns_and_never_blocks() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    assert!(d.contrast_warnings().is_empty());
    let text = d.theme_page.resolved.folder.default_text;
    d.set_element_colour(SchemeGroup::Folder, "background", text);
    let warnings = d.contrast_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("folder.background"), "{warnings:?}");
    assert!(d.problems().is_empty());
    assert!(enabled(&mut app, w, ButtonRole::Apply));
}

#[test]
fn syntax_elements_and_the_fallback_cycle_are_edited_and_reset() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("dark");
    let (name, style) = d
        .theme_page
        .resolved
        .file
        .syntax
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), *v))
        .expect("a syntax element");
    let mut changed = style;
    changed.color = RED;
    changed.bold = !style.bold;
    d.set_syntax(&name, changed);
    assert!(d.syntax_set(&name));
    assert_eq!(d.theme_page.resolved.file.syntax[&name], changed);
    d.set_fallback(3, RED);
    assert_eq!(d.theme_page.resolved.file.fallback[3], RED);
    assert_eq!(
        d.schemes["dark"].file.fallback.as_ref().map(Vec::len),
        Some(8)
    );
    d.set_syntax(&name, style);
    assert!(!d.syntax_set(&name), "equal to the base: nothing stored");
    d.set_syntax(&name, changed);
    d.reset_syntax(&name);
    d.reset_fallback();
    assert_eq!(d.schemes["dark"], Default::default());
}

// ---- A4.20 the wizards -----------------------------------------------------------------------------------------

fn wizard<T: crate::dialog::DialogModel>(app: &mut AppCore, w: WindowId) -> &mut T {
    app.window_mut(w)
        .expect("window")
        .dialogs
        .find_mut::<T>()
        .expect("the wizard")
}

fn fixed() -> impl FnOnce(&mut hikasa_session::ServiceInit) {
    |init| {
        use hikasa_session::Clock as _;
        init.clock = Arc::new(FixedClock(hikasa_session::SystemClock.now()));
    }
}

fn session(world: &World, name: &str) {
    let left = world.write("work/l.txt", "a\n");
    let right = world.write("work/r.txt", "b\n");
    let text = format!(
        "schema_version = 1\ntype = \"text-compare\"\n[specs]\nleft = {:?}\nright = {:?}\n",
        left.display().to_string(),
        right.display().to_string()
    );
    world.write(&format!("config/sessions/{name}.toml"), &text);
}

fn lock(world: &World, name: &str) {
    let path = world.path(&format!("config/sessions/{name}.toml"));
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, format!("locked = true\n{text}")).expect("write");
}

#[test]
fn the_export_wizard_equals_the_export_of_the_same_choices() {
    let world = World::with_init(fixed());
    session(&world, "one");
    session(&world, "two");
    world.write("config/workspaces/w.toml", "schema_version = 1\n");
    world
        .service
        .settings_set(
            &parse_key("options.tabs.hide_tab_bar_if_single_tab").expect("a key"),
            "true",
            false,
        )
        .expect("set");
    let mut app = world.app(None);
    let w = app.root();
    app.0.execute(w, CommandId::Export);
    world.settle(&mut app);
    let target = world.path("home/out.hikasa-pkg");
    let today = world.service.clock().now().strftime("%Y-%m-%d").to_string();
    let name = format!("hikasa-settings-{today}.hikasa-pkg");
    {
        let d = wizard::<ExportWizard>(&mut app, w);
        assert!(d.target.ends_with(&name), "{}", d.target);
        // Options and some sessions; nothing else.
        for row in &mut d.rows {
            row.checked = matches!(row.category, Category::Options | Category::Sessions);
            if row.category == Category::Sessions {
                row.all = false;
                for item in &mut row.items {
                    item.checked = item.name == "two";
                }
            }
        }
        d.target = target.display().to_string();
    }
    app.0.dialog_button(w, ButtonRole::Next);
    assert_eq!(wizard::<ExportWizard>(&mut app, w).step, 1);
    press(&world, &mut app, w, ButtonRole::Finish);
    let dto = wizard::<ExportWizard>(&mut app, w)
        .result
        .clone()
        .expect("a result");
    assert_eq!(wizard::<ExportWizard>(&mut app, w).step, 2);
    // The CLI's request of the same choices.
    let cli = world.path("home/cli.hikasa-pkg");
    let request = ExportRequest {
        categories: vec![Category::Options],
        items: BTreeMap::from([(Category::Sessions, vec!["two".to_owned()])]),
    };
    let expected = world
        .service
        .settings_export(&request, &cli)
        .expect("export");
    assert_eq!(dto.items, expected.items);
    assert_eq!(
        std::fs::read(&target).expect("package"),
        std::fs::read(&cli).expect("package")
    );
    press(&world, &mut app, w, ButtonRole::Close);
    assert_eq!(app.window(w).map(|x| x.dialogs.len()), Some(0));
}

#[test]
fn the_export_wizard_asks_before_replacing_a_file() {
    let world = World::new();
    let mut app = world.app(None);
    let w = app.root();
    app.0.execute(w, CommandId::Export);
    world.settle(&mut app);
    let target = world.write("home/x.hikasa-pkg", "old");
    wizard::<ExportWizard>(&mut app, w).target = target.display().to_string();
    app.0.dialog_button(w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Finish);
    assert!(top_prompt(&app, w).is_some_and(|p| p.message.contains("Replace")));
    assert_eq!(std::fs::read_to_string(&target).expect("kept"), "old");
    press(&world, &mut app, w, ButtonRole::Yes);
    assert_ne!(std::fs::read_to_string(&target).unwrap_or_default(), "old");
    assert_eq!(wizard::<ExportWizard>(&mut app, w).step, 2);
}

/// A package with sessions `a`, `b`, `c`, and a target configuration with `a` plain and `b` locked and `old`.
fn import_world(world: &World) -> std::path::PathBuf {
    let source = World::new();
    session(&source, "a");
    session(&source, "b");
    session(&source, "c");
    source.write("config/workspaces/w.toml", "schema_version = 1\n");
    let pkg = world.path("home/in.hikasa-pkg");
    source
        .service
        .settings_export(&ExportRequest::default(), &pkg)
        .expect("export");
    session(world, "a");
    session(world, "b");
    lock(world, "b");
    session(world, "old");
    pkg
}

#[test]
fn the_import_wizard_equals_the_import_of_the_same_choices() {
    let world = World::new();
    let twin = World::new();
    let pkg = import_world(&world);
    let twin_pkg = twin.path("home/in.hikasa-pkg");
    std::fs::copy(&pkg, &twin_pkg).expect("copy");
    session(&twin, "a");
    session(&twin, "b");
    lock(&twin, "b");
    session(&twin, "old");
    let mut app = world.app(None);
    let w = app.root();
    app.0.execute(
        w,
        Invocation::with(CommandId::Import, CommandArg::Paths(vec![pkg.clone()])),
    );
    assert_eq!(
        wizard::<ImportWizard>(&mut app, w).path,
        pkg.display().to_string(),
        "prefilled"
    );
    press(&world, &mut app, w, ButtonRole::Next);
    {
        let d = wizard::<ImportWizard>(&mut app, w);
        assert_eq!(d.step, 1);
        let sessions = d
            .rows
            .iter()
            .find(|r| r.category == Category::Sessions)
            .expect("sessions");
        let action = |name: &str| {
            sessions
                .items
                .iter()
                .find(|i| i.name == name)
                .and_then(|i| i.action)
        };
        assert_eq!(action("a"), Some(PlanAction::Replace));
        assert_eq!(action("b"), Some(PlanAction::SkipLocked));
        assert_eq!(action("c"), Some(PlanAction::Add));
    }
    // The summary of a first choice: b stays.
    press(&world, &mut app, w, ButtonRole::Next);
    let lines = {
        let d = wizard::<ImportWizard>(&mut app, w);
        assert_eq!(d.step, 2);
        summary_lines(d.summary.as_ref().expect("a summary"))
    };
    assert!(
        lines
            .iter()
            .any(|l| l == "2 sessions will be imported, replacing 1 existing."),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "1 session locked: kept as is."),
        "{lines:?}"
    );
    press(&world, &mut app, w, ButtonRole::Back);
    // Include locked sessions, delete the existing ones.
    {
        let d = wizard::<ImportWizard>(&mut app, w);
        d.force = true;
        d.delete_existing.insert(Category::Sessions);
        for row in &mut d.rows {
            row.checked = row.category == Category::Sessions;
            if row.category == Category::Sessions {
                row.all = false;
                for item in &mut row.items {
                    item.checked = item.name != "c";
                }
            }
        }
    }
    press(&world, &mut app, w, ButtonRole::Next);
    let lines = summary_lines(
        wizard::<ImportWizard>(&mut app, w)
            .summary
            .as_ref()
            .expect("a summary"),
    );
    golden("import-summary.txt", &(lines.join("\n") + "\n"));
    press(&world, &mut app, w, ButtonRole::Finish);
    let result = {
        let d = wizard::<ImportWizard>(&mut app, w);
        assert_eq!(d.step, 3);
        assert!(!d.plan_changed());
        d.result.clone().expect("a result")
    };
    let choices = ImportChoices {
        items: BTreeMap::from([(Category::Sessions, vec!["a".to_owned(), "b".to_owned()])]),
        delete_existing: BTreeSet::from([Category::Sessions]),
        force: true,
        ..ImportChoices::default()
    };
    let cli = twin
        .service
        .settings_import(&twin_pkg, &choices, false)
        .expect("import");
    assert_eq!(result.results, cli.results);
    assert_eq!(result.ok(), cli.ok());
    let names = |dir: &World| -> Vec<String> {
        files(dir)
            .into_keys()
            .filter(|k| k.starts_with("sessions"))
            .collect()
    };
    assert_eq!(names(&world), names(&twin));
    assert_eq!(names(&world), ["sessions/a.toml", "sessions/b.toml"]);
}

#[test]
fn a_summary_that_went_stale_is_reported_on_the_result() {
    let world = World::new();
    let pkg = import_world(&world);
    let mut app = world.app(None);
    let w = app.root();
    app.0.execute(
        w,
        Invocation::with(CommandId::Import, CommandArg::Paths(vec![pkg])),
    );
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Next);
    // Between the summary and Finish `c` appears: Add became Replace.
    session(&world, "c");
    press(&world, &mut app, w, ButtonRole::Finish);
    assert!(wizard::<ImportWizard>(&mut app, w).plan_changed());
}

#[test]
fn an_invalid_package_stops_at_step_one_with_nothing_written() {
    let world = World::new();
    let bad = world.write("home/bad.hikasa-pkg", "not a zip");
    let mut app = world.app(None);
    let before = files(&world);
    let w = app.root();
    app.0.execute(
        w,
        Invocation::with(CommandId::Import, CommandArg::Paths(vec![bad])),
    );
    press(&world, &mut app, w, ButtonRole::Next);
    let d = wizard::<ImportWizard>(&mut app, w);
    assert_eq!(d.step, 0);
    assert!(d.error.is_some());
    assert_eq!(files(&world), before);
}

#[test]
fn the_restore_wizard_equals_restore_defaults_by_category() {
    let world = World::new();
    let twin = World::new();
    for w in [&world, &twin] {
        w.write(
            "config/themes/x.toml",
            "schema_version = 1\nbase = \"dark\"\n",
        );
        w.write(
            "config/theme.toml",
            "schema_version = 1\nlight_scheme = \"x\"\n",
        );
        w.write(
            "config/commands.toml",
            "schema_version = 1\n[view.home]\nlarge_buttons = true\n",
        );
        w.write(
            "config/options.toml",
            "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n",
        );
        session(w, "keep");
        session(w, "gone");
    }
    lock(&world, "keep");
    lock(&twin, "keep");
    let mut app = world.app(None);
    let w = app.root();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    app.0.execute(w, CommandId::Restore);
    {
        let d = wizard::<RestoreWizard>(&mut app, w);
        d.categories
            .retain(|(c, _)| matches!(c, Category::Theme | Category::Commands));
        d.session_defaults = false;
        d.delete_sessions = true;
    }
    press(&world, &mut app, w, ButtonRole::Next);
    let plan = wizard::<RestoreWizard>(&mut app, w)
        .plan
        .clone()
        .expect("a plan");
    let cats = [Category::Theme, Category::Commands];
    let expected = twin
        .service
        .settings_restore_categories(&cats, true, false, false, true)
        .expect("plan");
    let rel = |d: &hikasa_session::api::SettingsRestoreDto, base: &Path| -> Vec<String> {
        d.files
            .iter()
            .map(|f| {
                Path::new(&f.path)
                    .strip_prefix(base)
                    .expect("inside")
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect()
    };
    assert_eq!(
        rel(&plan, &world.path("config")),
        rel(&expected, &twin.path("config"))
    );
    assert_eq!(plan.warnings.len(), 1, "the locked session is kept");
    press(&world, &mut app, w, ButtonRole::Finish);
    let d = wizard::<RestoreWizard>(&mut app, w);
    assert_eq!(d.step, 2);
    assert!(!d.plan_changed());
    twin.service
        .settings_restore_categories(&cats, true, false, false, false)
        .expect("restore");
    let names = |w: &World| -> Vec<String> { files(w).into_keys().collect() };
    assert_eq!(names(&world), names(&twin));
    assert_eq!(names(&world), ["options.toml", "sessions/keep.toml"]);
    // The settings reload with no notice for the GUI's own writes.
    assert_eq!(watcher.poll_once().expect("alive"), Vec::new());
    assert!(!app.tables().view(Scope::Home).custom.large_buttons);
    assert_eq!(app.theme().light_scheme, "light");
    assert!(app.options().tabs.hide_tab_bar_if_single_tab, "not chosen");
    assert!(
        app.window_mut(w)
            .is_some_and(|x| x.notices.visible(0.0).is_empty())
    );
}

#[test]
fn an_imported_format_reaches_the_service_and_the_home_tree_refreshes() {
    let world = World::new();
    let source = World::new();
    session(&source, "imported");
    source
        .service
        .format_new("my-c", Some("c-cpp"), None, None, None)
        .expect("a format");
    let pkg = world.path("home/in.hikasa-pkg");
    std::fs::create_dir_all(pkg.parent().expect("a parent")).expect("mkdir");
    source
        .service
        .settings_export(&ExportRequest::default(), &pkg)
        .expect("export");
    let mut app = world.app(None);
    world.settle(&mut app);
    assert!(world.service.format_show("my-c").is_err());
    let key = "imported";
    let rows = |app: &TestApp| {
        crate::tests_sessions::home(app)
            .tree
            .iter()
            .filter(|r| r.key.contains(key))
            .count()
    };
    assert_eq!(rows(&app), 0);
    let w = app.root();
    app.0.execute(
        w,
        Invocation::with(CommandId::Import, CommandArg::Paths(vec![pkg])),
    );
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Finish);
    press(&world, &mut app, w, ButtonRole::Close);
    world.settle(&mut app);
    // The running service took the format: no restart, no second read by hand.
    assert!(
        world
            .service
            .format_list()
            .formats
            .iter()
            .any(|f| f.id == "my-c"),
        "the service's registry"
    );
    assert_eq!(rows(&app), 1, "the Home tree read the sessions again");
}

// ---- A4.21 ImportPackage ---------------------------------------------------------------------------------------

#[test]
fn import_package_shows_the_wizard_prefilled_and_answers_ok() {
    let world = World::new();
    let pkg = world.write("home/p.hikasa-pkg", "x");
    let mut app = world.app(None);
    let second = {
        let w = app.root();
        app.0.execute(w, CommandId::WindowNew);
        app.windows()[1].id
    };
    app.focus(second);
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let response = handler.handle(
        Request::ImportPackage {
            path: pkg.display().to_string(),
        },
        None,
    );
    assert_eq!(response, Response::Ok);
    world.settle(&mut app);
    assert_eq!(
        wizard::<ImportWizard>(&mut app, second).path,
        pkg.display().to_string()
    );
    // A relative path is refused.
    let refused = handler.handle(
        Request::ImportPackage {
            path: "p.hikasa-pkg".into(),
        },
        None,
    );
    assert!(matches!(&refused, Response::Error { code, .. } if code == "invalid_request"));
}

#[test]
fn the_command_line_shows_the_wizard_after_the_first_frame() {
    let world = World::new();
    let pkg = world.write("home/p.hikasa-pkg", "x");
    let mut app = world.app(None);
    app.set_startup(crate::app::Startup::Import(pkg.clone()));
    app.after_first_frame();
    let w = app.root();
    assert_eq!(
        wizard::<ImportWizard>(&mut app, w).path,
        pkg.display().to_string()
    );
}

#[test]
fn a_package_argument_is_forwarded_and_silent_imports_headless() {
    use hikasa_session::api::{OpenArgs, SingleArg};
    let world = World::new();
    let pkg = world.path("home/p.hikasa-pkg");
    let source = World::new();
    session(&source, "imported");
    source
        .service
        .settings_export(&ExportRequest::default(), &pkg)
        .expect("export");
    assert_eq!(
        world.service.classify_single_arg(pkg.as_os_str()),
        SingleArg::Package(pkg.clone())
    );
    assert_eq!(crate::import_silently(&world.service, &pkg), 0);
    assert!(world.path("config/sessions/imported.toml").exists());
    let bad = world.write("home/bad.hikasa-pkg", "x");
    assert_eq!(crate::import_silently(&world.service, &bad), 100);
    // With a GUI running the package goes as `ImportPackage`.
    let endpoint = world.service.endpoint().expect("endpoint");
    let recorder = Arc::new(crate::tests_sessions::Recorder(std::sync::Mutex::new(
        Vec::new(),
    )));
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handle = server
        .spawn(Arc::clone(&recorder) as Arc<dyn IpcHandler>)
        .expect("spawn");
    let retry = hikasa_session::ipc::RetryPolicy::default();
    let single = SingleArg::Package(pkg.clone());
    assert_eq!(
        crate::forward(&world.service, &OpenArgs::default(), &single, &retry),
        0
    );
    drop(handle);
    assert_eq!(
        *recorder.0.lock().expect("lock"),
        vec![Request::ImportPackage {
            path: pkg.display().to_string()
        }]
    );
}

#[test]
fn a_view_keeps_running_through_a_restore() {
    use crate::tests_sessions::{make_dirty, r, way_world};
    // A named view whose session the restore deletes.
    let (world, mut app, view) = way_world(true);
    let w = app.root();
    app.0.execute(w, CommandId::Restore);
    world.settle(&mut app);
    {
        let d = wizard::<RestoreWizard>(&mut app, w);
        d.delete_sessions = true;
    }
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Finish);
    press(&world, &mut app, w, ButtonRole::Close);
    assert!(world.service.session_load(&r("main")).is_err(), "deleted");
    assert!(app.tab(view).is_some(), "the view runs on");
    // Saving reports that the session went; nothing else breaks, the edit stays in the view.
    make_dirty(&world, &mut app, view);
    app.0.execute(w, CommandId::SessionSave);
    world.settle(&mut app);
    let message = app.tab(view).and_then(|t| t.message.clone());
    assert!(message.is_some_and(|m| m.text.contains("main")));
    assert!(app.tab(view).is_some());
    // Its close asks about the unsaved change, then the view ends like any other.
    app.0.execute(w, CommandId::CloseTab);
    world.settle(&mut app);
    press(&world, &mut app, w, ButtonRole::No);
    assert!(app.tab(view).is_none());
}
