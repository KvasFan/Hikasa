//! Regression tests of the W4c review (§1.4.20 W4c): each names what it protects. Same harness as `tests_options`:
//! the toolkit-free models, no window.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hikasa_session::package::{Category, ExportRequest};
use hikasa_session::settings::theme::SchemeGroup;
use hikasa_session::settings::{Colour, FileId, FormValue, ValueKind, key_infos, parse_key};

use crate::app::{AppCore, Effect, UiMsg};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Prompt};
use crate::options::OptionsDialog;
use crate::options::commands_page::ViewFlag;
use crate::tests::{TestApp, World};
use crate::window::{CommandArg, Invocation, WindowId};
use crate::wizards::{ExportWizard, ImportWizard, delete_caption, summary_lines};

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
        .expect("the dialog")
}

fn has_dlg(app: &mut AppCore, w: WindowId) -> bool {
    app.window_mut(w)
        .is_some_and(|w| w.dialogs.find_mut::<OptionsDialog>().is_some())
}

fn press(world: &World, app: &mut TestApp, w: WindowId, role: ButtonRole) -> Vec<Effect> {
    let mut effects = app.0.dialog_button(w, role);
    effects.extend(world.settle(&mut app.0));
    effects
}

fn enabled(app: &mut AppCore, w: WindowId, role: ButtonRole) -> bool {
    dlg(app, w)
        .buttons()
        .iter()
        .any(|b| b.role == role && b.enabled)
}

fn top_prompt(app: &AppCore, w: WindowId) -> Option<Prompt> {
    app.window(w)?
        .dialogs
        .top()?
        .as_any()
        .downcast_ref::<Prompt>()
        .cloned()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn key(text: &str) -> hikasa_session::settings::Shortcut {
    hikasa_session::settings::Shortcut::parse(text).expect("a shortcut")
}

fn locked_themes(world: &World) -> std::path::PathBuf {
    let themes = world.path("config/themes");
    std::fs::create_dir_all(&themes).expect("mkdir");
    std::fs::set_permissions(&themes, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    themes
}

fn unlock(themes: &Path) {
    std::fs::set_permissions(themes, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

// ---- H1 what this build does not know blocks nothing and survives -----------------------------------------------

const FUTURE: &str = "schema_version = 1\n[view.future-view]\nlarge_buttons = true\n\
    [view.home.command.FutureCmd]\nmenu = false\n[view.home.command.FileFormatManager]\n\
    shortcuts = [\"Bogus+1\", \"Ctrl+F7\"]\n";

fn assert_future_kept(text: &str) {
    for kept in [
        "[view.future-view]",
        "[view.home.command.FutureCmd]",
        "[view.home.command.FileFormatManager]",
        "\"Bogus+1\"",
        "\"Ctrl+F7\"",
    ] {
        assert!(text.contains(kept), "{kept} lost:\n{text}");
    }
}

#[test]
fn an_unknown_id_disables_nothing_and_survives_apply_and_settings_set() {
    let world = World::new();
    let path = world.write("config/commands.toml", FUTURE);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    assert!(dlg(&mut app, w).problems().is_empty());
    dlg(&mut app, w).set_command_menu("About", false);
    assert!(dlg(&mut app, w).problems().is_empty());
    assert!(enabled(&mut app, w, ButtonRole::Apply));
    assert!(enabled(&mut app, w, ButtonRole::Ok));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(dlg(&mut app, w).message, None);
    let text = read(&path);
    assert_future_kept(&text);
    assert!(text.contains("[view.home.command.About]"), "{text}");
    // Reset View of a view whose planned override was set aside keeps that too.
    dlg(&mut app, w).reset_commands();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_future_kept(&read(&path));
    // The same input through the CLI's call.
    std::fs::write(&path, FUTURE).expect("write");
    world
        .service
        .settings_set(
            &parse_key("commands.view.home.large_buttons").expect("a key"),
            "true",
            false,
        )
        .expect("set");
    assert_future_kept(&read(&path));
}

/// Review T6: a bad element of `openwith.toml` or `filters.toml` blocks only an Apply of that file.
#[test]
fn a_bad_list_element_blocks_only_its_own_file() {
    let world = World::new();
    world.write(
        "config/openwith.toml",
        "schema_version = 1\n[[app]]\ndescription = \"x\"\ncommand = \"\"\n",
    );
    world.write(
        "config/filters.toml",
        "schema_version = 1\n[[preset]]\nname = \"a\"\nmask = \"[\"\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    assert_eq!(dlg(&mut app, w).problems(), &[] as &[String]);
    dlg(&mut app, w).set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    assert!(dlg(&mut app, w).problems().is_empty());
    assert!(enabled(&mut app, w, ButtonRole::Apply));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(read(&world.path("config/options.toml")).contains("open_new_tab_next_to_active"));
    // Editing the list brings its problem back: it is now part of what Apply would write.
    dlg(&mut app, w).preset_add();
    assert!(!dlg(&mut app, w).problems().is_empty());
    assert!(!enabled(&mut app, w, ButtonRole::Apply));
}

/// Review T5: a value the lenient load repairs reads the same through the dialog and through `settings set`.
#[test]
fn apply_over_repaired_values_equals_settings_set() {
    let text = "schema_version = 1\n[tweaks.metrics]\nmax_mru_items = 999999\n[tabs]\nopen_sessions_in = 5\n\
                hide_tab_bar_if_single_tab = false\nunknown_inner = 1\n";
    let world = World::new();
    let twin = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let path = world.write("config/options.toml", text);
    let other = twin.write("config/options.toml", text);
    let snapshot = world.service.settings_snapshot().expect("a snapshot");
    let w = app.root();
    let dialog = OptionsDialog::new(snapshot, None, Vec::new());
    app.0
        .window_mut(w)
        .expect("window")
        .dialogs
        .push(Box::new(dialog));
    dlg(&mut app, w).set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(top_prompt(&app, w), None);
    twin.service
        .settings_set(
            &parse_key("options.tabs.open_new_tab_next_to_active").expect("a key"),
            "false",
            false,
        )
        .expect("set");
    assert_eq!(read(&path), read(&other));
    assert!(read(&path).contains("unknown_inner = 1"));
}

// ---- H2 OK closes only when every file succeeded ------------------------------------------------------------------

#[test]
fn ok_with_a_failed_and_a_conflicted_file_keeps_the_dialog_and_the_failed_edit() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    d.select_scheme("light");
    d.set_element_colour(SchemeGroup::Folder, "orphan", Colour::hex(0x123456));
    world
        .service
        .settings_set(
            &parse_key("options.tabs.open_new_tab_next_to_active").expect("a key"),
            "false",
            false,
        )
        .expect("set");
    let themes = locked_themes(&world);
    press(&world, &mut app, w, ButtonRole::Ok);
    assert!(top_prompt(&app, w).is_some(), "the conflict prompt");
    press(&world, &mut app, w, ButtonRole::Other(1));
    unlock(&themes);
    assert!(has_dlg(&mut app.0, w), "the failed file is not saved");
    let d = dlg(&mut app, w);
    assert!(
        d.message.as_deref().is_some_and(|m| m.contains("light")),
        "{:?}",
        d.message
    );
    assert!(
        d.has_edits(&FileId::Themes("light".into())),
        "the edit is kept"
    );
    assert!(!d.has_edits(&FileId::Options));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(themes.join("light.toml").exists());
    assert_eq!(dlg(&mut app, w).message, None, "L1: the retry clears it");
}

#[test]
fn a_partial_failure_reports_each_file_and_ok_stays() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    d.select_scheme("light");
    d.set_element_colour(SchemeGroup::Folder, "orphan", Colour::hex(0x123456));
    let themes = locked_themes(&world);
    press(&world, &mut app, w, ButtonRole::Ok);
    unlock(&themes);
    assert!(has_dlg(&mut app.0, w));
    let d = dlg(&mut app, w);
    assert!(d.has_edits(&FileId::Themes("light".into())));
    assert!(
        !d.has_edits(&FileId::Options),
        "the written file is not an edit"
    );
    assert!(read(&world.path("config/options.toml")).contains("hide_tab_bar_if_single_tab"));
}

// ---- H3, L7 the override file of a built-in ---------------------------------------------------------------------

#[test]
fn resetting_the_last_element_keeps_a_file_with_unknown_keys() {
    let world = World::new();
    let path = world.write(
        "config/themes/light.toml",
        "schema_version = 1\nfuture_key = 1\n[folder]\norphan = \"#123456\"\nfuture_nested = 2\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.reset_element(SchemeGroup::Folder, "orphan");
    press(&world, &mut app, w, ButtonRole::Apply);
    let text = read(&path);
    assert!(text.contains("future_key = 1"), "{text}");
    assert!(text.contains("future_nested = 2"), "{text}");
    assert!(!text.contains("orphan"), "{text}");
    // *Reset all* is the explicit way to drop them.
    dlg(&mut app, w).reset_all();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(!path.exists());
}

#[test]
fn resetting_the_last_element_deletes_a_file_without_unknown_keys() {
    let world = World::new();
    let path = world.write(
        "config/themes/light.toml",
        "schema_version = 1\n[folder]\norphan = \"#123456\"\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.reset_element(SchemeGroup::Folder, "orphan");
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(!path.exists());
}

#[test]
fn a_later_edit_after_reset_all_does_not_delete_unknown_keys() {
    let world = World::new();
    let path = world.write(
        "config/themes/light.toml",
        "schema_version = 1\nfuture_key = 1\n[folder]\norphan = \"#123456\"\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.reset_all();
    d.set_element_colour(SchemeGroup::Folder, "orphan", Colour::hex(0x654321));
    d.reset_element(SchemeGroup::Folder, "orphan");
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(read(&path).contains("future_key = 1"));
}

#[test]
fn reset_all_deletes_despite_a_stray_base_key() {
    let world = World::new();
    let path = world.write(
        "config/themes/light.toml",
        "schema_version = 1\nbase = \"light\"\n[folder]\norphan = \"#123456\"\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.reset_all();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(!path.exists());
}

#[test]
fn unknown_keys_survive_a_scheme_save_and_a_delete_takes_them() {
    let world = World::new();
    let path = world.write(
        "config/themes/mine.toml",
        "schema_version = 1\nbase = \"dark\"\nfuture_key = 1\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("mine");
    d.set_element_colour(SchemeGroup::Folder, "orphan", Colour::hex(0x123456));
    press(&world, &mut app, w, ButtonRole::Apply);
    let text = read(&path);
    assert!(
        text.contains("future_key = 1") && text.contains("orphan"),
        "{text}"
    );
    let d = dlg(&mut app, w);
    d.select_scheme("mine");
    d.delete_scheme();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(!path.exists());
}

// ---- M1 a failed read-back is not a failed save -------------------------------------------------------------------

#[test]
fn a_snapshot_failure_after_the_writes_is_no_total_failure() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    let filters = world.write("config/filters.toml", "this is = = broken");
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(read(&world.path("config/options.toml")).contains("hide_tab_bar_if_single_tab"));
    let d = dlg(&mut app, w);
    assert!(
        d.message.as_deref().is_some_and(|m| m.contains("saved")),
        "{:?}",
        d.message
    );
    assert!(
        !d.has_edits(&FileId::Options),
        "written, so no edit is left"
    );
    // The next Apply meets no Conflict from the program's own write.
    std::fs::write(&filters, "schema_version = 1\n").expect("write");
    dlg(&mut app, w).set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(top_prompt(&app, w), None);
    assert_eq!(dlg(&mut app, w).message, None);
}

/// A file that cannot be read when the dialog opens does not stop the others.
#[test]
fn a_bad_unrelated_file_on_open_blocks_nothing() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    world.write("config/openwith.toml", "this is = = broken");
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    assert!(enabled(&mut app, w, ButtonRole::Apply));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(read(&world.path("config/options.toml")).contains("hide_tab_bar_if_single_tab"));
    assert_eq!(
        read(&world.path("config/openwith.toml")),
        "this is = = broken"
    );
}

// ---- M2 a second package ------------------------------------------------------------------------------------------

fn two_packages(world: &World) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = World::new();
    source.write(
        "config/sessions/fresh.toml",
        "schema_version = 1\ntype = \"text-compare\"\n[specs]\nleft = \"/a\"\nright = \"/b\"\n",
    );
    let (a, b) = (
        world.path("home/a.hikasa-pkg"),
        world.path("home/b.hikasa-pkg"),
    );
    std::fs::create_dir_all(a.parent().expect("a parent")).expect("mkdir");
    for pkg in [&a, &b] {
        source
            .service
            .settings_export(&ExportRequest::default(), pkg)
            .expect("export");
    }
    (a, b)
}

fn wizard(app: &mut AppCore, w: WindowId) -> &mut ImportWizard {
    app.window_mut(w)
        .expect("window")
        .dialogs
        .find_mut::<ImportWizard>()
        .expect("the wizard")
}

fn import(app: &mut TestApp, w: WindowId, pkg: &Path) {
    app.0.execute(
        w,
        Invocation::with(
            CommandId::Import,
            CommandArg::Paths(vec![pkg.to_path_buf()]),
        ),
    );
}

#[test]
fn a_second_package_after_step_one_asks_and_never_goes_ignored() {
    let world = World::new();
    let (a, b) = two_packages(&world);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    import(&mut app, w, &a);
    press(&world, &mut app, w, ButtonRole::Next);
    assert_eq!(wizard(&mut app.0, w).step, 1);
    // The package it shows already: nothing happens.
    world.mailbox.post(UiMsg::ImportPackage(a.clone()));
    world.settle(&mut app.0);
    assert_eq!(top_prompt(&app, w), None);
    // Another: a prompt that names both.
    world.mailbox.post(UiMsg::ImportPackage(b.clone()));
    world.settle(&mut app.0);
    let prompt = top_prompt(&app, w).expect("the question");
    assert!(prompt.message.contains("a.hikasa-pkg") && prompt.message.contains("b.hikasa-pkg"));
    // Keep: the wizard goes on with its package.
    press(&world, &mut app, w, ButtonRole::Cancel);
    assert!(wizard(&mut app.0, w).path.ends_with("a.hikasa-pkg"));
    assert_eq!(wizard(&mut app.0, w).step, 1);
    // Switch: a fresh wizard on the new package.
    world.mailbox.post(UiMsg::ImportPackage(b.clone()));
    world.settle(&mut app.0);
    press(&world, &mut app, w, ButtonRole::Yes);
    let d = wizard(&mut app.0, w);
    assert!(d.path.ends_with("b.hikasa-pkg"));
    assert_eq!(d.step, 0);
    assert!(d.rows.is_empty(), "no choice of the old package is left");
}

#[test]
fn a_package_during_a_job_raises_a_notice_and_after_the_result_opens_fresh() {
    let world = World::new();
    let (a, b) = two_packages(&world);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    import(&mut app, w, &a);
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Next);
    app.0.dialog_button(w, ButtonRole::Finish);
    assert!(wizard(&mut app.0, w).busy());
    app.0.handle(UiMsg::ImportPackage(b.clone()));
    let notices = &app.0.window(w).expect("window").notices;
    assert!(notices.get("import-busy").is_some());
    world.settle(&mut app.0);
    assert_eq!(wizard(&mut app.0, w).step, 3);
    assert!(wizard(&mut app.0, w).path.ends_with("a.hikasa-pkg"));
    // On the result page nothing is lost by moving on.
    app.0.handle(UiMsg::ImportPackage(b));
    assert_eq!(top_prompt(&app, w), None);
    let d = wizard(&mut app.0, w);
    assert_eq!(d.step, 0);
    assert!(d.path.ends_with("b.hikasa-pkg"));
}

// ---- M3 a close never cuts a write --------------------------------------------------------------------------------

#[test]
fn the_window_stays_while_an_import_runs_and_its_refresh_arrives() {
    let world = World::new();
    let (a, _) = two_packages(&world);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    import(&mut app, w, &a);
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Next);
    app.0.dialog_button(w, ButtonRole::Finish);
    assert!(wizard(&mut app.0, w).busy());
    // The OS close button and Exit, both refused with a notice.
    let effects = app.0.close_window(w);
    assert!(!effects.contains(&Effect::Exit));
    assert!(
        app.0
            .window(w)
            .expect("window")
            .dialogs
            .holds::<ImportWizard>()
    );
    assert!(
        app.0
            .window(w)
            .expect("window")
            .notices
            .get("dialog-busy")
            .is_some()
    );
    let effects = app.0.execute(w, CommandId::Exit);
    assert!(!effects.contains(&Effect::Exit));
    world.settle(&mut app.0);
    // It ended normally: result page, sessions on disk, and the window can close now.
    assert_eq!(wizard(&mut app.0, w).step, 3);
    assert!(world.path("config/sessions/fresh.toml").exists());
    press(&world, &mut app, w, ButtonRole::Close);
    assert!(app.0.close_window(w).contains(&Effect::Exit));
}

/// A dialog whose job takes a moment (the write of a shutdown nobody can refuse).
struct SlowWrite {
    done: Arc<AtomicBool>,
    requests: Vec<DialogRequest>,
}

impl DialogModel for SlowWrite {
    fn title(&self) -> String {
        "Slow".to_owned()
    }
    fn help_topic(&self) -> &'static str {
        "main-window"
    }
    fn buttons(&self) -> Vec<DialogButton> {
        Vec::new()
    }
    fn on_button(&mut self, _role: ButtonRole, _svc: &hikasa_session::Service) -> DialogStep {
        let done = Arc::clone(&self.done);
        self.requests.push(DialogRequest::Settings {
            title: "slow".to_owned(),
            body: Box::new(move |_| {
                std::thread::sleep(Duration::from_millis(150));
                done.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            }),
        });
        DialogStep::Stay
    }
    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn the_flush_of_an_exit_outwaits_a_settings_write() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    let done = Arc::new(AtomicBool::new(false));
    app.0
        .window_mut(w)
        .expect("window")
        .dialogs
        .push(Box::new(SlowWrite {
            done: Arc::clone(&done),
            requests: Vec::new(),
        }));
    app.0.dialog_button(w, ButtonRole::Ok);
    app.0.flush(Duration::ZERO);
    assert!(
        done.load(Ordering::SeqCst),
        "flush returned before the write ended"
    );
}

// ---- M4 Esc -------------------------------------------------------------------------------------------------------

#[test]
fn esc_with_edits_asks_and_without_them_closes() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    app.0.dialog_key(w, &key("Esc"));
    assert!(top_prompt(&app, w).is_some(), "asks first");
    assert!(has_dlg(&mut app.0, w));
    // Esc on the prompt: Keep editing.
    app.0.dialog_key(w, &key("Esc"));
    assert_eq!(top_prompt(&app, w), None);
    assert!(has_dlg(&mut app.0, w));
    assert!(dlg(&mut app, w).dirty(), "the edit is there");
    // Discard closes without a write.
    app.0.dialog_key(w, &key("Esc"));
    press(&world, &mut app, w, ButtonRole::Other(1));
    assert!(!has_dlg(&mut app.0, w));
    assert!(!world.path("config/options.toml").exists());
    // Nothing to lose: Esc closes at once.
    let w = open(&mut app);
    app.0.dialog_key(w, &key("Esc"));
    assert!(!has_dlg(&mut app.0, w));
}

// ---- M5 no path against the GUI's current folder --------------------------------------------------------------------

#[test]
fn a_relative_package_path_is_refused_and_nothing_lands_in_the_current_folder() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    app.0.execute(w, CommandId::Export);
    world.settle(&mut app.0);
    let name = "w4c-relative-target.hikasa-pkg";
    let d = app
        .0
        .window_mut(w)
        .expect("window")
        .dialogs
        .find_mut::<ExportWizard>()
        .expect("the wizard");
    d.target = name.to_owned();
    press(&world, &mut app, w, ButtonRole::Next);
    press(&world, &mut app, w, ButtonRole::Finish);
    let d = app
        .0
        .window_mut(w)
        .expect("window")
        .dialogs
        .find_mut::<ExportWizard>()
        .expect("the wizard");
    assert_eq!(d.step, 1);
    assert!(
        d.error.as_deref().is_some_and(|e| e.contains("full path")),
        "{:?}",
        d.error
    );
    assert!(!Path::new(name).exists());
    d.target = "~/x.hikasa-pkg".to_owned();
    press(&world, &mut app, w, ButtonRole::Finish);
    assert!(!Path::new("~").exists(), "nothing is expanded or created");
    press(&world, &mut app, w, ButtonRole::Cancel);
    app.0.execute(w, CommandId::Import);
    wizard(&mut app.0, w).path = "relative.hikasa-pkg".to_owned();
    press(&world, &mut app, w, ButtonRole::Next);
    let d = wizard(&mut app.0, w);
    assert_eq!(d.step, 0);
    assert!(d.error.as_deref().is_some_and(|e| e.contains("full path")));
}

// ---- L items --------------------------------------------------------------------------------------------------------

#[test]
fn a_retry_that_succeeds_clears_the_message() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_scheme("light");
    d.set_element_colour(SchemeGroup::Folder, "orphan", Colour::hex(0x123456));
    let themes = locked_themes(&world);
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(
        dlg(&mut app, w).message.is_some(),
        "FileOutcome::Failed is shown"
    );
    unlock(&themes);
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(dlg(&mut app, w).message, None);
    assert!(themes.join("light.toml").exists());
}

#[test]
fn a_capture_ends_when_the_page_is_left() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.show_page("commands");
    d.select_command("About");
    d.begin_capture();
    assert!(d.capturing());
    d.show_page("tabs");
    assert!(!d.capturing());
    assert!(!d.wants_keys());
}

#[test]
fn a_reload_asked_during_a_save_runs_after_it() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    app.0.dialog_button(w, ButtonRole::Apply);
    assert!(dlg(&mut app, w).saving());
    world.write(
        "config/archives.toml",
        "schema_version = 1\n[types]\nzip = \"*.zipped\"\n",
    );
    dlg(&mut app, w).reload_files(vec![FileId::Archives]);
    world.settle(&mut app.0);
    let d = dlg(&mut app, w);
    assert!(!d.saving());
    assert_eq!(
        d.archives.mask(hikasa_session::settings::ArchiveType::Zip),
        "*.zipped"
    );
}

#[test]
fn an_unedited_file_changed_during_a_save_gets_its_stamp_refreshed() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    dlg(&mut app, w).set_field("tabs.hide_tab_bar_if_single_tab", FormValue::Bool(true));
    // Another program writes a file the dialog has no edits for, before the job reads back.
    world.write(
        "config/filters.toml",
        "schema_version = 1\n[[preset]]\nname = \"other\"\nmask = \"*.o\"\n",
    );
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(dlg(&mut app, w).filters.preset.len(), 1);
    // An edit of that file saves without a false conflict.
    dlg(&mut app, w).preset_add();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert_eq!(top_prompt(&app, w), None);
    assert!(read(&world.path("config/filters.toml")).contains("Preset 2"));
}

#[test]
fn a_dialog_that_goes_with_its_window_ends_the_preview() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tabs]\nclosing_last_tab_closes_window = true\n",
    )]);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let root = app.root();
    app.0.execute(root, CommandId::WindowNew);
    world.settle(&mut app.0);
    let second = app.0.window_ids()[1];
    app.0.execute(second, CommandId::ProgramOptions);
    let d = dlg(&mut app, second);
    d.select_scheme("light");
    d.set_font_size(crate::options::theme_page::FontSlot::Editor, 40.0);
    app.0.dialog_edited(second);
    assert_eq!(app.0.theme().fonts.editor.size, 40.0);
    // The last tab of the window goes: the window goes with its dialog.
    app.0.close_window_now(second);
    world.settle(&mut app.0);
    assert!(app.0.window(second).is_none());
    assert_ne!(app.0.theme().fonts.editor.size, 40.0, "the preview ended");
}

#[test]
fn readding_a_default_shortcut_writes_nothing_and_default_is_off_for_a_menu_override() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.select_command("Exit");
    let shortcut = d
        .commands_page
        .rows
        .iter()
        .find(|r| r.id == "Exit")
        .and_then(|r| r.shortcuts.first().cloned())
        .expect("Exit has a shortcut");
    d.commands_page.shortcut = Some(0);
    d.remove_shortcut();
    assert!(d.commands.view["home"].command["Exit"].shortcuts.is_some());
    d.begin_capture();
    d.capture(&shortcut);
    assert!(d.commands.view.is_empty(), "{:?}", d.commands);
    assert!(!d.dirty());
    // A menu-only override: `Default` has nothing to do, so it is off.
    d.set_command_menu("About", false);
    let row = d
        .commands_page
        .rows
        .iter()
        .find(|r| r.id == "About")
        .expect("a row");
    assert!(!row.customized);
}

#[test]
fn apply_takes_the_open_item_or_refuses_it() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    // A bad item: Apply refuses, the panel stays, nothing is dropped.
    let d = dlg(&mut app, w);
    d.open_with_new();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(!world.path("config/openwith.toml").exists());
    let d = dlg(&mut app, w);
    assert!(d.open_with.editing.is_some());
    assert_eq!(d.page, "open_with");
    assert!(d.message.is_some());
    // Filled in: Apply takes it.
    let edit = d.open_with.editing.as_mut().expect("the panel");
    edit.app.description = "Ed".to_owned();
    edit.app.command = "ed %f".to_owned();
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(read(&world.path("config/openwith.toml")).contains("Ed"));
    assert!(dlg(&mut app, w).open_with.editing.is_none());
    assert_eq!(dlg(&mut app, w).message, None);
}

#[test]
fn an_open_item_keeps_its_row_through_a_follow() {
    let two = "schema_version = 1\n[[app]]\ndescription = \"A\"\ncommand = \"a %f\"\n\
               [[app]]\ndescription = \"B\"\ncommand = \"b %f\"\n";
    let world = World::new();
    world.write("config/openwith.toml", two);
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.open_with.selected = Some(1);
    d.open_with_edit();
    assert!(
        d.has_edits(&FileId::OpenWith),
        "an open item counts as an edit"
    );
    world.write(
        "config/openwith.toml",
        "schema_version = 1\n[[app]]\ndescription = \"B\"\ncommand = \"b %f\"\n",
    );
    let fresh = world.service.settings_snapshot().expect("a snapshot");
    let d = dlg(&mut app, w);
    d.follow(&fresh);
    assert_eq!(d.openwith.app.len(), 2, "the list under the panel stays");
    assert_eq!(d.open_with.editing.as_ref().and_then(|e| e.index), Some(1));
}

#[test]
fn the_summary_names_locked_sessions_kept_by_delete_existing() {
    let world = World::new();
    let (a, _) = two_packages(&world);
    world.write(
        "config/sessions/mine.toml",
        "locked = true\nschema_version = 1\ntype = \"text-compare\"\n[specs]\nleft = \"/a\"\nright = \"/b\"\n",
    );
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let w = app.root();
    import(&mut app, w, &a);
    press(&world, &mut app, w, ButtonRole::Next);
    wizard(&mut app.0, w)
        .delete_existing
        .insert(Category::Sessions);
    press(&world, &mut app, w, ButtonRole::Next);
    let d = wizard(&mut app.0, w);
    let lines = summary_lines(d.summary.as_ref().expect("the summary"));
    assert!(
        lines.iter().any(|l| l == "1 session locked: kept as is."),
        "{lines:?}"
    );
    press(&world, &mut app, w, ButtonRole::Finish);
    assert!(world.path("config/sessions/mine.toml").exists());
}

// ---- conformance ----------------------------------------------------------------------------------------------------

#[test]
fn delete_choices_read_as_captions() {
    assert_eq!(
        delete_caption(Category::Sessions),
        "Delete existing sessions"
    );
    assert_eq!(
        delete_caption(Category::Formats),
        "Delete existing file formats"
    );
    assert_eq!(
        delete_caption(Category::Workspaces),
        "Delete existing workspaces"
    );
}

#[test]
fn the_open_with_shortcut_is_a_capture_field() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    d.open_with_new();
    d.open_with_begin_capture();
    assert!(d.wants_keys());
    app.0.dialog_key(w, &key("Ctrl+F9"));
    let d = dlg(&mut app, w);
    assert!(!d.capturing());
    let edit = d.open_with.editing.as_ref().expect("the panel");
    assert_eq!(edit.app.shortcut, "Ctrl+F9");
    // Esc gives up and keeps what was there, without closing anything.
    d.open_with_begin_capture();
    app.0.dialog_key(w, &key("Esc"));
    let d = dlg(&mut app, w);
    assert_eq!(
        d.open_with
            .editing
            .as_ref()
            .expect("the panel")
            .app
            .shortcut,
        "Ctrl+F9"
    );
}

/// Review T7: every P0 key has a field and a widget that can edit it.
#[test]
fn every_p0_key_has_a_field_with_a_real_widget() {
    let world = World::new();
    let mut app = world.app(None);
    let w = open(&mut app);
    let d = dlg(&mut app, w);
    for info in key_infos(&FileId::Options) {
        if info.priority != Some("P0") {
            continue;
        }
        let path = info.key.strip_prefix("options.").expect("the prefix");
        let field = d.field(path).unwrap_or_else(|| panic!("{path}: no field"));
        assert!(
            matches!(
                (&field.value, field.kind),
                (FormValue::Bool(_), _)
                    | (FormValue::Int(_), ValueKind::Int { .. })
                    | (FormValue::Text(_), _)
            ),
            "{path}: {:?} {:?} has no widget",
            field.value,
            field.kind
        );
    }
}

/// Review T8: a configuration the window could not read: the dialog shows defaults and nothing is overwritten
/// without the conflict prompt.
#[test]
fn defaults_never_replace_an_unreadable_file_without_asking() {
    let world = World::new();
    let mut app = world.app(None);
    world.settle(&mut app.0);
    let options = world.write(
        "config/options.toml",
        "schema_version = 99\nfuture = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n",
    );
    let before = read(&options);
    let w = open(&mut app);
    dlg(&mut app, w).set_view_flag(ViewFlag::ShowAllInMenu, true);
    dlg(&mut app, w).set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    press(&world, &mut app, w, ButtonRole::Apply);
    assert!(top_prompt(&app, w).is_some());
    assert_eq!(read(&options), before);
}
