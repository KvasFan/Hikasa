//! M2 W4a (§1.4.15): the GUI's settings calls `settings_snapshot`, `settings_save`, `reload_settings`,
//! `state_update`, `log_append`, `input_stamp` and the input watch, in a temporary configuration.

mod common;

use std::path::Path;

use common::{service, write};
use hikasa_session::settings::{
    ChangeKind, CommandOverrides, FileId, FileOutcome, LoadMode, SettingsChanges, SettingsFile,
    State, parse_key,
};
use hikasa_session::watch::{InputState, InputWatch};

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn a_snapshot_reads_every_file_leniently() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    // The service reads `options.toml` strictly: the bad value is written after it started.
    let svc = service(tmp.path());
    write(
        &config,
        "options.toml",
        b"schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n[tweaks.metrics]\nmax_mru_items = 0\n",
    );
    write(
        &config,
        "themes/mine.toml",
        b"schema_version = 1\nbase = \"dark\"\n",
    );
    let snap = svc.settings_snapshot().unwrap();
    assert!(snap.options.value().tabs.hide_tab_bar_if_single_tab);
    // Lenient: the bad value is its default, with a warning.
    assert_eq!(snap.options.value().tweaks.metrics.max_mru_items, 20);
    assert!(
        snap.warnings()
            .iter()
            .any(|w| w.message.contains("max_mru_items")),
        "{:?}",
        snap.warnings()
    );
    let names: Vec<&str> = snap.schemes.keys().map(String::as_str).collect();
    assert_eq!(names, ["dark", "light", "mine"]);
    // A scheme that does not exist falls back to the built-in of its mode.
    let mut theme = snap.theme.value().clone();
    theme.dark_scheme = "gone".into();
    let mut snap2 = snap.clone();
    *snap2.theme.value_mut() = theme;
    use hikasa_session::settings::ThemeMode;
    assert_eq!(snap2.scheme(ThemeMode::FollowOs, true).name, "dark");
    assert_eq!(snap2.scheme(ThemeMode::Light, true).name, "light");
    // A newer schema is an error.
    write(&config, "commands.toml", b"schema_version = 9\n");
    assert!(svc.settings_snapshot().is_err());
}

#[test]
fn settings_save_writes_only_changed_files_like_settings_set() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    // An unknown key is kept (A1.2).
    write(
        &config,
        "options.toml",
        b"schema_version = 1\nfuture_key = 1\n[tabs]\nfuture = \"x\"\n",
    );
    let svc = service(tmp.path());
    let snap = svc.settings_snapshot().unwrap();
    // No change: nothing written, nothing reported.
    let none = svc
        .settings_save(&snap, &SettingsChanges::default(), false)
        .unwrap();
    assert!(none.files.is_empty());
    // A change equal to the snapshot is `Unchanged`.
    let same = SettingsChanges {
        options: Some(snap.options.value().clone()),
        ..SettingsChanges::default()
    };
    let out = svc.settings_save(&snap, &same, false).unwrap();
    assert_eq!(out.files.len(), 1);
    assert_eq!(out.files[0].2, FileOutcome::Unchanged);
    assert!(!config.join("commands.toml").exists());
    // One change writes only that file, sparsely, unknown keys kept; the bytes equal `settings set`'s.
    let mut options = snap.options.value().clone();
    options.tabs.open_new_tab_next_to_active = false;
    let changes = SettingsChanges {
        options: Some(options),
        ..SettingsChanges::default()
    };
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    assert_eq!(
        out.files,
        vec![(
            FileId::Options,
            config.join("options.toml"),
            FileOutcome::Written
        )]
    );
    assert!(
        !out.snapshot
            .options
            .value()
            .tabs
            .open_new_tab_next_to_active
    );
    let ours = read(&config.join("options.toml"));
    assert!(ours.contains("future_key = 1"), "{ours}");
    assert!(
        ours.contains("open_new_tab_next_to_active = false"),
        "{ours}"
    );
    assert!(!ours.contains("warn_when_closing"), "sparse: {ours}");
    for f in ["commands.toml", "theme.toml", "filters.toml", "state.toml"] {
        assert!(!config.join(f).exists(), "{f} written");
    }
    // The same edit through `settings set` on a twin configuration gives the same bytes.
    let twin = tempfile::tempdir().unwrap();
    write(
        &twin.path().join("config"),
        "options.toml",
        b"schema_version = 1\nfuture_key = 1\n[tabs]\nfuture = \"x\"\n",
    );
    let twin_svc = service(twin.path());
    twin_svc
        .settings_set(
            &parse_key("options.tabs.open_new_tab_next_to_active").unwrap(),
            "false",
            false,
        )
        .unwrap();
    assert_eq!(read(&twin.path().join("config/options.toml")), ours);
    // An invalid value is not written.
    let mut bad = out.snapshot.options.value().clone();
    bad.tweaks.metrics.max_mru_items = 0;
    let out2 = svc
        .settings_save(
            &out.snapshot,
            &SettingsChanges {
                options: Some(bad),
                ..SettingsChanges::default()
            },
            false,
        )
        .unwrap();
    assert!(
        matches!(&out2.files[0].2, FileOutcome::Invalid(p) if p[0].key.contains("max_mru_items"))
    );
    assert_eq!(read(&config.join("options.toml")), ours);
}

#[test]
fn settings_save_reports_conflicts_and_deletes_schemes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    write(
        &config,
        "themes/mine.toml",
        b"schema_version = 1\nbase = \"dark\"\n",
    );
    let svc = service(tmp.path());
    let snap = svc.settings_snapshot().unwrap();
    // A concurrent CLI edit: `Conflict` unless forced.
    svc.settings_set(
        &parse_key("commands.view.home.large_buttons").unwrap(),
        "true",
        false,
    )
    .unwrap();
    let mut commands = snap.commands.value().clone();
    commands
        .view
        .entry("text-compare".into())
        .or_default()
        .show_all_in_menu = true;
    let changes = SettingsChanges {
        commands: Some(commands),
        ..SettingsChanges::default()
    };
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Conflict);
    assert!(read(&config.join("commands.toml")).contains("large_buttons"));
    let out = svc.settings_save(&snap, &changes, true).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Written);
    let text = read(&config.join("commands.toml"));
    assert!(
        text.contains("show_all_in_menu = true") && !text.contains("large_buttons"),
        "{text}"
    );
    // Scheme files: change one, delete one, delete one that does not exist.
    let snap = out.snapshot;
    let mut mine = snap.schemes["mine"].value().clone();
    mine.folder.same = Some(hikasa_session::settings::Colour::parse("#123456").unwrap());
    let mut changes = SettingsChanges::default();
    changes.schemes.insert("dark".into(), None);
    changes.schemes.insert("mine".into(), Some(mine));
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    let outcomes: Vec<(&FileId, &FileOutcome)> = out.files.iter().map(|(f, _, o)| (f, o)).collect();
    assert_eq!(
        outcomes,
        vec![
            (&FileId::Themes("dark".into()), &FileOutcome::Unchanged),
            (&FileId::Themes("mine".into()), &FileOutcome::Written),
        ]
    );
    assert!(read(&config.join("themes/mine.toml")).contains("#123456"));
    let mut changes = SettingsChanges::default();
    changes.schemes.insert("mine".into(), None);
    let out = svc.settings_save(&out.snapshot, &changes, false).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Deleted);
    assert!(!config.join("themes/mine.toml").exists());
    assert!(!out.snapshot.schemes.contains_key("mine"));
}

#[test]
fn reload_settings_reaches_later_calls_only() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let svc = service(tmp.path());
    let before = svc.options();
    let registry_before = svc.registry();
    assert_eq!(before.context_lines, 3);
    write(
        &config,
        "options.toml",
        b"schema_version = 1\n[text_editing]\nnumber_of_context_lines = 7\n[tweaks.metrics]\nmax_mru_items = 0\n",
    );
    svc.format_new("w4fmt", None, Some("W4 format"), Some("*.w4"), None)
        .unwrap();
    assert!(svc.registry().find("w4fmt").is_none(), "no reload yet");
    let warnings = svc.reload_settings();
    assert!(
        warnings.iter().any(|w| w.message.contains("max_mru_items")),
        "{warnings:?}"
    );
    assert_eq!(svc.options().context_lines, 7);
    assert!(svc.registry().find("w4fmt").is_some());
    // A snapshot taken before (a running job's) keeps the old values.
    assert_eq!(before.context_lines, 3);
    assert!(registry_before.find("w4fmt").is_none());
    // A file that cannot be read keeps the previous values, with a warning.
    write(&config, "options.toml", b"schema_version = 9\n");
    let warnings = svc.reload_settings();
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("previous settings")),
        "{warnings:?}"
    );
    assert_eq!(svc.options().context_lines, 7);
}

#[test]
fn state_update_merges_concurrent_edits_and_skips_no_ops() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let svc = service(tmp.path());
    let path = config.join("state.toml");
    // Nothing to change: nothing written.
    assert!(!svc.state_update(&mut |_s: &mut State| {}).unwrap());
    assert!(!path.exists());
    // A layout table and the window.
    let wrote = svc
        .state_update(&mut |s: &mut State| {
            let mut t = toml::Table::new();
            t.insert("font_size".into(), toml::Value::Float(15.0));
            t.insert("thumbnail".into(), toml::Value::Boolean(false));
            s.layout.insert("text-compare".into(), t);
            s.window.width = 1000;
        })
        .unwrap();
    assert!(wrote);
    let text = read(&path);
    assert!(text.contains("[layout.text-compare]"), "{text}");
    assert!(text.contains("font_size = 15.0"), "{text}");
    // `settings get state.layout.text-compare` reads it.
    let got = svc
        .settings_get(&parse_key("state.layout.text-compare").unwrap())
        .unwrap();
    assert_eq!(got.value["font_size"], serde_json::json!(15.0));
    let got = svc
        .settings_get(&parse_key("state.layout.text-compare.thumbnail").unwrap())
        .unwrap();
    assert_eq!(got.value, serde_json::json!(false));
    // The same change again: nothing written.
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(
        !svc.state_update(&mut |s: &mut State| s.window.width = 1000)
            .unwrap()
    );
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    // An edit between the load and the save (a concurrent `settings set`): the retry applies both.
    let mut calls = 0;
    let svc2 = svc.clone();
    svc.state_update(&mut |s: &mut State| {
        calls += 1;
        if calls == 1 {
            svc2.settings_set(&parse_key("state.last_session").unwrap(), "\"cli\"", false)
                .unwrap();
        }
        s.window.height = 600;
    })
    .unwrap();
    assert_eq!(calls, 2);
    let state = svc.settings_store().state(LoadMode::Strict).unwrap().value;
    assert_eq!(state.last_session, "cli");
    assert_eq!(state.window.height, 600);
    assert_eq!(state.window.width, 1000);
    assert!(state.layout.contains_key("text-compare"));
}

#[test]
fn log_append_creates_and_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let path = tmp.path().join("logs/deep/hikasa.log");
    svc.log_append(&path, &["one".into(), "two".into()])
        .unwrap();
    svc.log_append(&path, &["three".into()]).unwrap();
    assert_eq!(read(&path), "one\ntwo\nthree\n");
    assert!(
        svc.log_append(&tmp.path().join("logs"), &["x".into()])
            .is_err()
    );
}

#[test]
fn input_stamps_and_the_input_watch() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let file = write(tmp.path(), "work/a.txt", b"abc");
    let spec = file.to_string_lossy().into_owned();
    let InputState::Present(stamp) = svc.input_stamp(&spec) else {
        panic!("present");
    };
    assert_eq!(stamp.len, 3);
    assert!(stamp.modified_ns.is_some());
    // Relative specs resolve against the captured working folder.
    assert_eq!(svc.input_stamp("a.txt"), InputState::Present(stamp));
    for blob in ["-", "clipboard://"] {
        assert_eq!(svc.input_stamp(blob), InputState::Unwatched);
    }
    assert_eq!(svc.input_stamp("gone.txt"), InputState::Missing);
    // The spooled copy of standard input is a blob too.
    let spool = svc
        .config()
        .cache_dir
        .join("stdin")
        .join("1")
        .join("stdin.txt");
    std::fs::create_dir_all(spool.parent().unwrap()).unwrap();
    std::fs::write(&spool, b"from stdin").unwrap();
    assert_eq!(
        svc.input_stamp(&spool.to_string_lossy()),
        InputState::Unwatched
    );

    let mut watch = InputWatch::new();
    watch.track(spec.clone(), svc.input_stamp(&spec));
    watch.track("-".into(), svc.input_stamp("-"));
    assert_eq!(watch.specs().collect::<Vec<_>>(), vec![spec.as_str()]);
    assert!(watch.poll(&svc).is_empty());
    std::fs::write(&file, b"abcd").unwrap();
    assert_eq!(watch.poll(&svc), vec![(spec.clone(), ChangeKind::Changed)]);
    assert!(watch.poll(&svc).is_empty());
    std::fs::remove_file(&file).unwrap();
    assert_eq!(watch.poll(&svc), vec![(spec.clone(), ChangeKind::Removed)]);
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(watch.poll(&svc), vec![(spec.clone(), ChangeKind::Created)]);
    watch.untrack(&spec);
    std::fs::write(&file, b"xyz12").unwrap();
    assert!(watch.poll(&svc).is_empty());
}

#[test]
fn the_input_watch_counts_the_views_of_an_input() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let file = write(tmp.path(), "work/a.txt", b"abc");
    let spec = file.to_string_lossy().into_owned();
    let mut watch = InputWatch::new();
    // Two views of one input (a duplicate, or two opens of the same pair).
    watch.track(spec.clone(), svc.input_stamp(&spec));
    std::fs::write(&file, b"abcd").unwrap();
    // The second track keeps the first stamp: the change not polled yet is still reported.
    watch.track(spec.clone(), svc.input_stamp(&spec));
    assert!(!watch.retain("other"));
    assert_eq!(watch.poll(&svc), vec![(spec.clone(), ChangeKind::Changed)]);
    // Closing one view keeps the input watched for the other.
    watch.untrack(&spec);
    assert!(watch.is_tracked(&spec));
    std::fs::write(&file, b"abcde").unwrap();
    assert_eq!(watch.poll(&svc), vec![(spec.clone(), ChangeKind::Changed)]);
    // Closing the last one stops it; an untrack too many changes nothing.
    watch.untrack(&spec);
    watch.untrack(&spec);
    assert!(!watch.is_tracked(&spec));
    std::fs::write(&file, b"x").unwrap();
    assert!(watch.poll(&svc).is_empty());
    // Blobs never count.
    watch.track("-".into(), svc.input_stamp("-"));
    assert!(!watch.is_tracked("-"));
}

#[test]
fn reload_settings_swaps_one_consistent_view_and_serializes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let svc = service(tmp.path());
    assert!(svc.ignored_option_keys().is_empty());
    // The ignored keys are refreshed with the options.
    write(
        &config,
        "options.toml",
        b"schema_version = 1\n[text_editing]\nnumber_of_context_lines = 5\nunknown_5 = 1\n",
    );
    svc.reload_settings();
    let view = svc.loaded_settings();
    assert_eq!(view.options.context_lines, 5);
    assert_eq!(view.ignored_keys, svc.ignored_option_keys());
    assert!(
        svc.ignored_option_keys()
            .iter()
            .any(|k| k.contains("unknown_5")),
        "{:?}",
        svc.ignored_option_keys()
    );
    // Concurrent reloads while the file changes: once the last write is followed by reloads, every reload that
    // finishes later sees it (no reload started earlier finishes after them with older values), and readers
    // always get options and ignored keys of one load.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let svc = svc.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let v = svc.loaded_settings();
                    let n = v.options.context_lines;
                    let marker = format!("unknown_{n}");
                    assert!(
                        v.ignored_keys.iter().any(|k| k.contains(&marker)),
                        "{n} {:?}",
                        v.ignored_keys
                    );
                }
            })
        })
        .collect();
    for round in 0..20u32 {
        let n = 1 + round % 9;
        let body = format!(
            "schema_version = 1\n[text_editing]\nnumber_of_context_lines = {n}\nunknown_{n} = 1\n"
        );
        let reloaders: Vec<_> = (0..3)
            .map(|_| {
                let svc = svc.clone();
                std::thread::spawn(move || {
                    svc.reload_settings();
                })
            })
            .collect();
        // Written whole (rename), so a reload never reads half a file.
        let next = write(&config, "options.next", body.as_bytes());
        std::fs::rename(&next, config.join("options.toml")).unwrap();
        let last = {
            let svc = svc.clone();
            std::thread::spawn(move || {
                svc.reload_settings();
            })
        };
        for r in reloaders {
            r.join().unwrap();
        }
        last.join().unwrap();
        assert_eq!(svc.options().context_lines as u32, n, "round {round}");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in readers {
        r.join().unwrap();
    }
}

/// Commands of a later build, and a planned command's override with a shortcut this build cannot read (H1).
const FUTURE_COMMANDS: &str = "schema_version = 1\n[view.future-view]\nlarge_buttons = true\n\
    [view.home.command.FutureCmd]\nmenu = false\n[view.home.command.FileFormatManager]\n\
    shortcuts = [\"Bogus+1\", \"Ctrl+F7\"]\n";

fn assert_future_commands_kept(text: &str) {
    for key in [
        "[view.future-view]",
        "large_buttons = true",
        "[view.home.command.FutureCmd]",
        "menu = false",
        "[view.home.command.FileFormatManager]",
        "\"Bogus+1\"",
        "\"Ctrl+F7\"",
    ] {
        assert!(text.contains(key), "{key} lost:\n{text}");
    }
}

#[test]
fn unknown_views_and_commands_are_kept_with_warnings_not_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    write(&config, "commands.toml", FUTURE_COMMANDS.as_bytes());
    let svc = service(tmp.path());
    let snap = svc.settings_snapshot().unwrap();
    // Out of the value, in the kept keys: nothing to validate, nothing repaired.
    assert!(
        snap.commands.value().view.is_empty(),
        "{:?}",
        snap.commands.value()
    );
    let warnings = snap.commands.loaded().warnings.clone();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.message.contains("future-view")));
    assert!(warnings.iter().any(|w| w.message.contains("FutureCmd")));
    assert!(warnings.iter().any(|w| w.message.contains("Bogus+1")));
    assert!(
        snap.commands
            .loaded()
            .ignored_keys
            .contains(&"view.future-view.large_buttons".to_owned())
    );
    // A strict read (a package entry) takes them too.
    SettingsFile::<CommandOverrides>::parse(FUTURE_COMMANDS, "c", LoadMode::Strict).unwrap();
    // `settings_save` of a change elsewhere in the file keeps them.
    let mut commands = snap.commands.value().clone();
    commands.set_in_menu("home", "About", Some(false));
    let changes = SettingsChanges {
        commands: Some(commands),
        ..SettingsChanges::default()
    };
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Written);
    let saved = read(&config.join("commands.toml"));
    assert_future_commands_kept(&saved);
    assert!(saved.contains("[view.home.command.About]"), "{saved}");
    // The file reads back the same: a second save has nothing to do.
    let again = svc.settings_snapshot().unwrap();
    assert_eq!(
        again.commands.value().view["home"]
            .command
            .keys()
            .collect::<Vec<_>>(),
        ["About"]
    );
    assert_eq!(
        read(&config.join("commands.toml")),
        again.commands.to_toml()
    );
}

#[test]
fn settings_set_keeps_unknown_views_commands_and_planned_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    write(&config, "commands.toml", FUTURE_COMMANDS.as_bytes());
    let svc = service(tmp.path());
    let key = parse_key("commands.view.home.large_buttons").unwrap();
    let dto = svc.settings_set(&key, "true", false).unwrap();
    assert!(dto.written);
    let text = read(&config.join("commands.toml"));
    assert_future_commands_kept(&text);
    assert!(text.contains("[view.home]\nlarge_buttons = true"), "{text}");
    // Typing a new key there stays refused.
    let key = parse_key("commands.view.future-view.show_all_in_menu").unwrap();
    assert!(svc.settings_set(&key, "true", false).is_err());
}

#[test]
fn a_failed_read_back_keeps_the_outcomes_and_the_saved_stamps() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let svc = service(tmp.path());
    let snap = svc.settings_snapshot().unwrap();
    // Any file that no longer parses fails the snapshot after the write.
    write(&config, "filters.toml", b"this is = = broken");
    let mut options = snap.options.value().clone();
    options.tabs.open_new_tab_next_to_active = false;
    let changes = SettingsChanges {
        options: Some(options.clone()),
        ..SettingsChanges::default()
    };
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Written);
    assert!(
        out.reread_error
            .as_deref()
            .is_some_and(|e| e.contains("filters.toml"))
    );
    // The stale snapshot holds what was written, with its stamp: the next save is no Conflict.
    assert_eq!(*out.snapshot.options.value(), options);
    assert!(!out.snapshot.options.changed_on_disk().unwrap());
}

#[test]
fn every_file_of_a_save_has_its_own_outcome() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let svc = service(tmp.path());
    let snap = svc.settings_snapshot().unwrap();
    let mut options = snap.options.value().clone();
    options.tabs.open_new_tab_next_to_active = false;
    let mut scheme = snap.schemes["light"].value().clone();
    scheme.base = Some(hikasa_session::settings::theme::SchemeBase::Dark);
    let mut changes = SettingsChanges {
        options: Some(options),
        ..SettingsChanges::default()
    };
    changes.schemes.insert("light".into(), Some(scheme));
    let themes = config.join("themes");
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::set_permissions(&themes, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = svc.settings_save(&snap, &changes, false).unwrap();
    std::fs::set_permissions(&themes, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.files[0].2, FileOutcome::Written);
    assert!(
        matches!(out.files[1].2, FileOutcome::Failed(_)),
        "{:?}",
        out.files
    );
    assert!(out.reread_error.is_none());
}
