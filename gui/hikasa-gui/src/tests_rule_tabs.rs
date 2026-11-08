//! M2 W9c acceptance A9.23 (D-249) and A9.24 (D-250): the folder rule tabs of Session Settings and the toolbar's
//! Filters edit, through the models. Each tab shows its P0 keys; an edit written with a scope equals `hikasa session
//! edit --set` of the same key (files compared); invalid values keep OK off with core's message on their tab; the Misc
//! tab's bold rows are the session's overrides; *Disable editing* round-trips; `Rules` and `NameFilter` open their
//! tabs; OK runs the view again; P1 keys survive. The Filters edit shows core's canonical text, takes presets and the
//! recent filters, refuses a bad mask, pushes `mru.name_filters`; *Add To Presets* writes `filters.toml` (sparse,
//! unknown keys kept), refuses a name in use, offers W4c's choices on a concurrent edit, and the Options page sees it.

use hikasa_core::ContentMethod;
use hikasa_folder::sync::{Situation, SyncAction};
use hikasa_folder::{ArchiveHandling, CaseMode};
use hikasa_session::api::{SessionEdit, SessionTarget};
use hikasa_session::sessions::{Scope as RuleScope, SessionRef, SessionRules, SessionType};

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::filters_edit::PresetNameDialog;
use crate::folder_tabs::{
    COMPARISON, ComparisonTab, HandlingTab, MiscTab, NAME_FILTERS, NameFiltersTab, SyncTab,
};
use crate::session_dialog::SessionSettingsDialog;
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{open, put, shown};

fn fixture(world: &World) {
    put(world, "l", "a.txt", "left a\n", 3_000);
    put(world, "r", "a.txt", "right a\n", 1_000);
    put(world, "l", "b.rs", "fn b() {}\n", 1_000);
    put(world, "r", "b.rs", "fn b() {}\n", 1_000);
}

fn dialog(app: &mut TestApp) -> &mut SessionSettingsDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<SessionSettingsDialog>())
        .expect("Session Settings")
}

fn tab<T: crate::session_dialog::SessionTab>(app: &mut TestApp) -> &mut T {
    dialog(app).tab_mut::<T>().expect("the tab")
}

/// Opens Session Settings of the active view with `command` (`Rules`, `NameFilter`, `SessionEdit`).
fn settings(world: &World, app: &mut TestApp, command: CommandId) {
    app.execute(command);
    world.settle(app);
}

fn ok(world: &World, app: &mut TestApp) {
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(app);
}

#[test]
fn a9_23_rules_and_name_filter_open_their_tabs_with_the_p0_keys() {
    let world = World::new();
    fixture(&world);
    // A defaults file with a value for each tab's keys.
    let mut defaults = world
        .service
        .session_defaults(SessionType::FolderCompare)
        .expect("defaults");
    if let SessionRules::Folder(r) = &mut defaults.rules {
        r.criteria.size = false;
        r.criteria.timestamps.tolerance_secs = 7;
        r.criteria.content.method = ContentMethod::Binary;
        r.filters.names.include_files = vec!["*.rs".into(), "*.txt".into()];
        r.alignment.case = CaseMode::Insensitive;
        r.handling.archives = ArchiveHandling::Always;
        r.ignored = vec!["x".into()];
        r.format_overrides.insert("c".into(), false);
    }
    world
        .service
        .session_save_defaults(&defaults)
        .expect("defaults");
    let mut app = world.app(None);
    let _view = open(&world, &mut app);
    settings(&world, &mut app, CommandId::Rules);
    let d = dialog(&mut app);
    let titles: Vec<String> = d.tabs.iter().map(|t| t.title()).collect();
    assert_eq!(
        titles,
        ["Specs", "Comparison", "Name Filters", "Handling", "Misc"]
    );
    assert_eq!(d.tabs[d.active].title(), COMPARISON);
    let c = tab::<ComparisonTab>(&mut app).fields.clone();
    assert!(!c.size);
    assert_eq!(c.tolerance, "7");
    assert_eq!(c.method, ContentMethod::Binary);
    let n = tab::<NameFiltersTab>(&mut app);
    assert_eq!(n.fields.include_files, "*.rs\n*.txt");
    assert_eq!(n.text().as_deref(), Ok("*.rs;*.txt"));
    assert!(!n.presets.is_empty(), "the presets of filters.toml");
    let h = tab::<HandlingTab>(&mut app).fields;
    assert_eq!(
        (h.case, h.archives),
        (CaseMode::Insensitive, ArchiveHandling::Always)
    );
    let m = tab::<MiscTab>(&mut app);
    assert_eq!(m.fields.ignored, ["x"]);
    assert!(!m.formats.is_empty(), "format list");
    // Bold rows are the session's overrides.
    let bold: Vec<String> = m
        .rows()
        .into_iter()
        .filter(|r| r.session.is_some())
        .map(|r| r.id)
        .collect();
    let overrides: Vec<String> = m.fields.overrides.keys().cloned().collect();
    let known: Vec<String> = overrides
        .iter()
        .filter(|id| m.formats.iter().any(|(f, _, _)| f == *id))
        .cloned()
        .collect();
    assert_eq!(bold, known);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Cancel);
    settings(&world, &mut app, CommandId::NameFilter);
    let d = dialog(&mut app);
    assert_eq!(d.tabs[d.active].title(), NAME_FILTERS);
}

/// One P0 key edited on its tab with the Defaults scope, and the same key through `session edit --set` on a twin
/// (a word of the vocabulary is written as core serializes it, a TOML string).
type Edit = (&'static str, String, fn(&mut SessionSettingsDialog));

/// The serde word of a value, as a TOML string.
fn word<T: serde::Serialize>(value: T) -> String {
    serde_json::to_string(&value).expect("a word")
}

fn edits() -> Vec<Edit> {
    let plain = |v: &str| v.to_owned();
    vec![
        ("rules.criteria.size", plain("false"), |d| {
            d.tab_mut::<ComparisonTab>().expect("tab").fields.size = false;
        }),
        (
            "rules.criteria.timestamps.tolerance_secs",
            plain("5"),
            |d| {
                d.tab_mut::<ComparisonTab>().expect("tab").fields.tolerance = "5".into();
            },
        ),
        ("rules.criteria.filename_case", plain("true"), |d| {
            d.tab_mut::<ComparisonTab>()
                .expect("tab")
                .fields
                .filename_case = true;
        }),
        (
            "rules.criteria.content.method",
            word(ContentMethod::Binary),
            |d| {
                d.tab_mut::<ComparisonTab>().expect("tab").fields.method = ContentMethod::Binary;
            },
        ),
        ("rules.alignment.align_extensions", plain("true"), |d| {
            d.tab_mut::<HandlingTab>()
                .expect("tab")
                .fields
                .align_extensions = true;
        }),
        ("rules.handling.follow_symlinks", plain("true"), |d| {
            d.tab_mut::<HandlingTab>()
                .expect("tab")
                .fields
                .follow_symlinks = true;
        }),
        ("rules.handling.scan_orphan_folders", plain("false"), |d| {
            d.tab_mut::<HandlingTab>()
                .expect("tab")
                .fields
                .scan_orphan_folders = false;
        }),
        (
            "rules.handling.archives",
            word(ArchiveHandling::Always),
            |d| {
                d.tab_mut::<HandlingTab>().expect("tab").fields.archives = ArchiveHandling::Always;
            },
        ),
        ("rules.alignment.case", word(CaseMode::Insensitive), |d| {
            d.tab_mut::<HandlingTab>().expect("tab").fields.case = CaseMode::Insensitive;
        }),
        (
            "rules.filters.names.exclude_files",
            plain("[\"*.bak\"]"),
            |d| {
                d.tab_mut::<NameFiltersTab>()
                    .expect("tab")
                    .fields
                    .exclude_files = "*.bak".into();
            },
        ),
    ]
}

/// Every file below the configuration's sessions folder: path → content.
fn sessions_files(world: &World) -> std::collections::BTreeMap<String, String> {
    let root = world.path("config/sessions");
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(&root)
                    .expect("below")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}

#[test]
fn a9_23_an_edit_with_the_defaults_scope_is_session_edit_set() {
    for (key, value, edit) in edits() {
        let (gui, twin) = (World::new(), World::new());
        fixture(&gui);
        let mut app = gui.app(None);
        let _view = open(&gui, &mut app);
        settings(&gui, &mut app, CommandId::Rules);
        let d = dialog(&mut app);
        edit(d);
        assert!(d.scope_available(RuleScope::Defaults), "{key}");
        d.scope = RuleScope::Defaults;
        ok(&gui, &mut app);
        twin.service
            .session_edit(
                &SessionTarget::Defaults(SessionType::FolderCompare),
                &SessionEdit {
                    set: vec![(key.to_owned(), value.clone())],
                    ..SessionEdit::default()
                },
                false,
            )
            .expect("session edit");
        assert_eq!(sessions_files(&gui), sessions_files(&twin), "{key}");
    }
}

/// What the view holds on its own (a *this view only* value, an Ignored path, a sync row's override) stays out of the
/// defaults when a later edit is written with the Defaults scope: the file equals `session edit --set` of the key
/// edited alone (W9c review F2).
#[test]
fn a9_23_values_the_view_holds_on_its_own_stay_out_of_the_defaults() {
    for start in ["this view only", "an ignored path", "a sync override"] {
        let (gui, twin) = (World::new(), World::new());
        fixture(&gui);
        let mut app = gui.app(None);
        let sync = start == "a sync override";
        let view = if sync {
            crate::tests_sync_view::open_sync(
                &gui,
                &mut app.0,
                hikasa_folder::sync::SyncOptions::default(),
            )
        } else {
            open(&gui, &mut app)
        };
        match start {
            "this view only" => {
                settings(&gui, &mut app, CommandId::Rules);
                tab::<ComparisonTab>(&mut app).fields.tolerance = "9".into();
                dialog(&mut app).scope = RuleScope::View;
                ok(&gui, &mut app);
            }
            "an ignored path" => {
                crate::tests_folder_ops::select(&mut app.0, view, "a.txt");
                app.execute(CommandId::Ignored);
                gui.settle(&mut app);
                assert_eq!(
                    folder_model(&mut app.0, view).ignored_list(),
                    Some(&["a.txt".to_owned()][..])
                );
            }
            _ => {
                crate::tests_folder_ops::select(&mut app.0, view, "a.txt");
                app.execute(CommandId::SyncLeave);
                for _ in 0..4 {
                    app.0.handle(crate::app::UiMsg::Tick);
                    gui.settle(&mut app);
                }
                assert!(
                    folder_model(&mut app.0, view)
                        .sync()
                        .is_some_and(|s| s.options.overrides.contains_key("a.txt")),
                    "the override"
                );
            }
        }
        settings(&gui, &mut app, CommandId::Rules);
        tab::<ComparisonTab>(&mut app).fields.size = false;
        assert!(
            dialog(&mut app).scope_available(RuleScope::Defaults),
            "{start}"
        );
        dialog(&mut app).scope = RuleScope::Defaults;
        ok(&gui, &mut app);
        let kind = if sync {
            SessionType::FolderSync
        } else {
            SessionType::FolderCompare
        };
        twin.service
            .session_edit(
                &SessionTarget::Defaults(kind),
                &SessionEdit {
                    set: vec![("rules.criteria.size".into(), "false".into())],
                    ..SessionEdit::default()
                },
                false,
            )
            .expect("session edit");
        assert_eq!(sessions_files(&gui), sessions_files(&twin), "{start}");
        // The view keeps its own value and takes the edit.
        app.execute(CommandId::FavCompareAll);
        gui.settle(&mut app);
        let options = folder_model(&mut app.0, view)
            .prepared()
            .expect("a comparison")
            .rules()
            .clone();
        assert!(!options.criteria.size, "{start}");
        match start {
            "this view only" => assert_eq!(options.criteria.timestamps.tolerance_secs, 9),
            "an ignored path" => assert_eq!(options.ignored, ["a.txt"]),
            _ => {}
        }
    }
}

/// Edits on several tabs at once are all written (each tab writes its keys over the others').
#[test]
fn a9_23_edits_of_several_tabs_are_written_together() {
    let (gui, twin) = (World::new(), World::new());
    fixture(&gui);
    let mut app = gui.app(None);
    let _view = open(&gui, &mut app);
    settings(&gui, &mut app, CommandId::Rules);
    let all = edits();
    for (_, _, edit) in &all {
        edit(dialog(&mut app));
    }
    dialog(&mut app).scope = RuleScope::Defaults;
    ok(&gui, &mut app);
    twin.service
        .session_edit(
            &SessionTarget::Defaults(SessionType::FolderCompare),
            &SessionEdit {
                set: all
                    .iter()
                    .map(|(key, value, _)| ((*key).to_owned(), value.clone()))
                    .collect(),
                ..SessionEdit::default()
            },
            false,
        )
        .expect("session edit");
    assert_eq!(sessions_files(&gui), sessions_files(&twin));
}

#[test]
fn a9_23_an_edit_of_a_named_view_saved_is_session_edit_set() {
    let (gui, twin) = (World::new(), World::new());
    let reference = SessionRef::parse("main").expect("ref");
    for world in [&gui, &twin] {
        fixture(world);
        let mut session = hikasa_session::sessions::Session::builtin(SessionType::FolderCompare);
        session.specs.left = world.path("work/l").display().to_string();
        session.specs.right = world.path("work/r").display().to_string();
        world
            .service
            .session_save(&reference, &session, false)
            .expect("save");
    }
    let mut app = gui.app(None);
    gui.settle(&mut app);
    let target = crate::app::OpenTarget::Window(app.win());
    app.0
        .open_named(reference.clone(), crate::app::OpenHow::Target(target));
    gui.settle(&mut app);
    settings(&gui, &mut app, CommandId::Rules);
    dialog(&mut app)
        .tab_mut::<ComparisonTab>()
        .expect("tab")
        .fields
        .tolerance = "9".into();
    dialog(&mut app).scope = RuleScope::View;
    ok(&gui, &mut app);
    app.execute(CommandId::SessionSave);
    gui.settle(&mut app);
    twin.service
        .session_edit(
            &SessionTarget::Named(reference),
            &SessionEdit {
                set: vec![(
                    "rules.criteria.timestamps.tolerance_secs".into(),
                    "9".into(),
                )],
                ..SessionEdit::default()
            },
            false,
        )
        .expect("session edit");
    // The rules saved are the same, and the file holds them as sparsely (the view's `[display]` and `[view]` are the
    // window's own and stay out of the comparison).
    let load = |world: &World| {
        world
            .service
            .session_load(&SessionRef::parse("main").expect("ref"))
            .expect("load")
            .value
    };
    assert_eq!(load(&gui).rules, load(&twin).rules);
    let rules_text = |world: &World| -> String {
        let text = std::fs::read_to_string(
            world
                .service
                .session_store()
                .path(&SessionRef::parse("main").expect("ref")),
        )
        .expect("read");
        let mut keep = false;
        let mut out = Vec::new();
        for line in text.lines() {
            if line.starts_with('[') {
                keep = line.starts_with("[rules");
            }
            if keep {
                out.push(line.to_owned());
            }
        }
        out.join("\n").trim_end().to_owned()
    };
    assert_eq!(rules_text(&gui), rules_text(&twin));
    assert!(rules_text(&gui).contains("tolerance_secs = 9"));
}

#[test]
fn a9_23_invalid_values_keep_ok_off_with_the_message_on_their_tab() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let _view = open(&world, &mut app);
    settings(&world, &mut app, CommandId::Rules);
    let poll = |world: &World, app: &mut TestApp| {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    };
    let ok_enabled = |app: &mut TestApp| {
        dialog(app)
            .buttons()
            .iter()
            .any(|b| b.role == ButtonRole::Ok && b.enabled)
    };
    // A tolerance out of range.
    tab::<ComparisonTab>(&mut app).fields.tolerance = "-3".into();
    poll(&world, &mut app);
    assert!(tab::<ComparisonTab>(&mut app).error.is_some());
    assert!(!ok_enabled(&mut app));
    tab::<ComparisonTab>(&mut app).fields.tolerance = "3".into();
    poll(&world, &mut app);
    assert!(tab::<ComparisonTab>(&mut app).error.is_none());
    assert!(ok_enabled(&mut app));
    // An offset out of -12..=12: core's message.
    let c = tab::<ComparisonTab>(&mut app);
    c.fields.offset_side = Some(hikasa_core::Side::Left);
    c.fields.offset_hours = "13".into();
    poll(&world, &mut app);
    let error = tab::<ComparisonTab>(&mut app)
        .error
        .clone()
        .expect("refused");
    assert!(error.contains("-12..=12"), "{error}");
    tab::<ComparisonTab>(&mut app).fields.offset_side = None;
    // A mask that does not compile: on the Name Filters tab; OK stays and shows that tab.
    tab::<NameFiltersTab>(&mut app).fields.exclude_folders = "[a".into();
    poll(&world, &mut app);
    assert!(tab::<NameFiltersTab>(&mut app).error.is_some());
    assert!(tab::<ComparisonTab>(&mut app).error.is_none());
    dialog(&mut app).active = 0;
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let d = dialog(&mut app);
    assert_eq!(
        d.tabs[d.active].title(),
        NAME_FILTERS,
        "OK shows the refusal"
    );
}

/// A tab other than the one shown holds an error: its title is marked and OK's tip names it (W9c review F11).
#[test]
fn an_error_on_another_tab_is_marked_and_named() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let _view = open(&world, &mut app);
    settings(&world, &mut app, CommandId::Rules);
    tab::<NameFiltersTab>(&mut app).fields.exclude_folders = "[a".into();
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let d = dialog(&mut app);
    assert_eq!(d.tabs[d.active].title(), COMPARISON, "another tab is shown");
    let k = d
        .tabs
        .iter()
        .position(|t| t.title() == NAME_FILTERS)
        .expect("the tab");
    assert_eq!(d.tab_caption(k), format!("{NAME_FILTERS} (!)"));
    assert_eq!(d.tab_caption(d.active), COMPARISON);
    let tip = d.button_tip(ButtonRole::Ok).expect("a reason");
    assert!(tip.starts_with(&format!("The {NAME_FILTERS} tab")), "{tip}");
    assert!(d.button_tip(ButtonRole::Cancel).is_none());
    d.tabs[k]
        .as_any_mut()
        .downcast_mut::<NameFiltersTab>()
        .expect("tab")
        .fields
        .exclude_folders = String::new();
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let d = dialog(&mut app);
    assert_eq!(d.tab_caption(k), NAME_FILTERS);
    assert!(d.button_tip(ButtonRole::Ok).is_none());
}

#[test]
fn a9_23_a_bad_matrix_cell_is_refused_and_the_sync_tab_edits_the_options() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = crate::tests_sync_view::open_sync(
        &world,
        &mut app.0,
        hikasa_folder::sync::SyncOptions::default(),
    );
    let _ = view;
    settings(&world, &mut app, CommandId::Rules);
    let titles: Vec<String> = dialog(&mut app).tabs.iter().map(|t| t.title()).collect();
    assert_eq!(
        titles.last().map(String::as_str),
        Some(crate::folder_tabs::sync_title())
    );
    // Invalid cells are not offered: core's rule.
    assert!(!SyncTab::allows(Situation::AddedLeft, SyncAction::CopyLeft));
    assert!(SyncTab::allows(Situation::AddedLeft, SyncAction::CopyRight));
    let t = tab::<SyncTab>(&mut app);
    t.set_cell(Situation::AddedLeft, SyncAction::CopyLeft);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    assert!(
        tab::<SyncTab>(&mut app).error.is_some(),
        "the matrix refuses it"
    );
    let t = tab::<SyncTab>(&mut app);
    t.set_cell(Situation::AddedLeft, SyncAction::Leave);
    t.fields.options.create_empty = true;
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    assert!(tab::<SyncTab>(&mut app).error.is_none());
    ok(&world, &mut app);
    // The view runs with the custom matrix and the option.
    let active = app.tabs()[app.active()].id;
    let options = folder_model(&mut app.0, active)
        .sync()
        .map(|s| s.options.clone())
        .expect("a sync view");
    assert_eq!(options.preset, hikasa_folder::sync::SyncPreset::Custom);
    assert!(options.create_empty);
    assert_eq!(options.matrix.get(Situation::AddedLeft), SyncAction::Leave);
}

#[test]
fn a9_23_disable_editing_ok_reruns_and_p1_keys_survive() {
    let world = World::new();
    fixture(&world);
    // A session file with P1 keys (DOS attributes, Unix tests, a size filter).
    let reference = SessionRef::parse("p1").expect("ref");
    let mut session = hikasa_session::sessions::Session::builtin(SessionType::FolderCompare);
    session.specs.left = world.path("work/l").display().to_string();
    session.specs.right = world.path("work/r").display().to_string();
    if let SessionRules::Folder(r) = &mut session.rules {
        r.criteria.dos_attributes = hikasa_vfs::DosAttrs::parse_letters("HR").expect("letters");
        r.criteria.unix.permissions = true;
        r.filters.other.size =
            Some(hikasa_folder::SizeFilter::parse("<10MB").expect("a size filter"));
    }
    world
        .service
        .session_save(&reference, &session, false)
        .expect("save");
    let mut app = world.app(None);
    world.settle(&mut app);
    let target = crate::app::OpenTarget::Window(app.win());
    app.0
        .open_named(reference.clone(), crate::app::OpenHow::Target(target));
    world.settle(&mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    let view = app.tabs()[app.active()].id;
    assert!(shown(&mut app.0, view).iter().any(|(p, _)| p == "a.txt"));
    // *Disable editing* of the right side, and a name filter that hides the text files.
    settings(&world, &mut app, CommandId::SessionEdit);
    dialog(&mut app).specs_mut().read_only[1] = true;
    tab::<NameFiltersTab>(&mut app).fields.exclude_files = "*.txt".into();
    ok(&world, &mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    // OK ran the view again with the filter, the right side read-only.
    assert!(!shown(&mut app.0, view).iter().any(|(p, _)| p == "a.txt"));
    assert!(
        !folder_model(&mut app.0, view)
            .write_policy()
            .writable(hikasa_core::Side::Right)
    );
    // The dialog shows it again.
    settings(&world, &mut app, CommandId::SessionEdit);
    assert_eq!(dialog(&mut app).specs().read_only, [false, true]);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Cancel);
    // Saved: the P1 keys are as they were.
    app.execute(CommandId::SessionSave);
    world.settle(&mut app);
    let saved = world.service.session_load(&reference).expect("load").value;
    let SessionRules::Folder(rules) = saved.rules else {
        panic!("folder rules")
    };
    assert_eq!(rules.criteria.dos_attributes.letters(), "HR");
    assert!(rules.criteria.unix.permissions);
    assert!(rules.filters.other.size.is_some());
    assert_eq!(rules.filters.names.exclude_files, ["*.txt"]);
    assert!(saved.specs.read_only_right);
}

#[test]
fn a9_23_the_misc_tab_toggles_and_resets_overrides() {
    let mut t = MiscTab::default();
    t.prepare(&crate::session_dialog::TabExtras {
        formats: vec![
            ("c".into(), "C".into(), true),
            ("x".into(), "X".into(), false),
        ],
        presets: Vec::new(),
    });
    t.load(&SessionRules::Folder(hikasa_folder::FolderRules {
        format_overrides: [("gone".to_owned(), true)].into_iter().collect(),
        ..hikasa_folder::FolderRules::default()
    }));
    t.set_enabled("c", false);
    t.set_enabled("x", false);
    assert_eq!(
        t.rows()
            .iter()
            .map(|r| (r.id.as_str(), r.session))
            .collect::<Vec<_>>(),
        [("c", Some(false)), ("x", None)]
    );
    t.set_enabled("c", true);
    assert_eq!(
        t.rows()[0].session,
        None,
        "back to the global state: no override"
    );
    t.set_enabled("x", true);
    t.reset("x");
    assert!(t.rows().iter().all(|r| r.session.is_none()));
    // An override of a format the registry does not list is kept as stored.
    assert_eq!(t.fields.overrides.keys().collect::<Vec<_>>(), ["gone"]);
}

use crate::session_dialog::SessionTab as _;

// ---- A9.24: the Filters edit and presets --------------------------------------------------------------------------

#[test]
fn a9_24_the_filters_edit_shows_applies_and_remembers() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    assert_eq!(folder_model(&mut app.0, view).filters_edit().text, "");
    let w = app.win();
    // A bad mask: core's message under the field, nothing runs.
    folder_model(&mut app.0, view).filters_edit().text = "[a".into();
    app.0.filters_enter(w, None);
    world.settle(&mut app);
    assert!(
        folder_model(&mut app.0, view)
            .filters_edit()
            .error
            .is_some()
    );
    assert!(shown(&mut app.0, view).iter().any(|(p, _)| p == "a.txt"));
    // A valid one: the view runs with it; the field shows core's canonical text (D-07's order); the MRU has it.
    folder_model(&mut app.0, view).filters_edit().text = "-*.txt;*.rs".into();
    app.0.filters_enter(w, None);
    world.settle(&mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    let rows = shown(&mut app.0, view);
    assert!(rows.iter().any(|(p, _)| p == "b.rs"));
    assert!(!rows.iter().any(|(p, _)| p == "a.txt"));
    assert_eq!(
        folder_model(&mut app.0, view).filters_edit().text,
        "*.rs;-*.txt"
    );
    let (presets, recent) = app.0.filters_choices();
    assert_eq!(recent.first().map(String::as_str), Some("*.rs;-*.txt"));
    // A preset of the drop-down.
    let (_, mask) = presets.first().cloned().expect("a preset");
    app.0.filters_enter(w, Some(&mask));
    world.settle(&mut app);
    let expected = crate::folder_tabs::toolbar_text(
        &hikasa_core::NameFilter::parse(&mask, crate::folder_tabs::TEXT_OPTIONS)
            .expect("mask")
            .to_lists(),
    )
    .expect("text");
    assert_eq!(folder_model(&mut app.0, view).filters_edit().text, expected);
    // A recent one.
    app.0.filters_enter(w, Some("*.rs;-*.txt"));
    world.settle(&mut app);
    assert_eq!(
        folder_model(&mut app.0, view).filters_edit().text,
        "*.rs;-*.txt"
    );
}

const FILTERS_FILE: &str =
    "schema_version = 1\nfuture_key = \"kept\"\n\n[[preset]]\nname = \"Mine\"\nmask = \"*.c\"\n";

fn preset_dialog(app: &mut TestApp) -> &mut PresetNameDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<PresetNameDialog>())
        .expect("Add To Presets")
}

#[test]
fn a9_24_add_to_presets_writes_filters_toml() {
    let world = World::with_files(&[("filters.toml", FILTERS_FILE)]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    folder_model(&mut app.0, view).filters_edit().text = "*.rs;-*.bak".into();
    let w = app.win();
    app.0.filters_add_to_presets(w);
    // A name in use is refused at once, nothing is written.
    preset_dialog(&mut app).name = "Mine".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    assert!(
        preset_dialog(&mut app).error.is_some(),
        "refused before any write"
    );
    assert!(!preset_dialog(&mut app).busy());
    world.settle(&mut app);
    assert!(preset_dialog(&mut app).error.is_some());
    preset_dialog(&mut app).name = "Rust".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let w = app.win();
    assert!(
        app.0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<PresetNameDialog>())
            .is_none(),
        "closed after the write"
    );
    let text = std::fs::read_to_string(world.path("config/filters.toml")).expect("filters.toml");
    crate::tests_shell::golden("filters-add-to-presets.toml", &text);
    // The Options page of OPT-020 sees it.
    world.settle(&mut app);
    app.execute(CommandId::ProgramOptions);
    world.settle(&mut app);
    let names: Vec<String> = {
        let w = app.win();
        let d = app
            .0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<crate::options::OptionsDialog>())
            .expect("Options");
        d.filters.preset.iter().map(|p| p.name.clone()).collect()
    };
    assert_eq!(names, ["Mine", "Rust"]);
}

#[test]
fn a9_24_a_concurrent_edit_offers_overwrite_reload_cancel() {
    let world = World::with_files(&[("filters.toml", FILTERS_FILE)]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    folder_model(&mut app.0, view).filters_edit().text = "*.rs".into();
    let w = app.win();
    app.0.filters_add_to_presets(w);
    preset_dialog(&mut app).name = "Rust".into();
    // The CLI changes the file meanwhile.
    std::fs::write(
        world.path("config/filters.toml"),
        format!("{FILTERS_FILE}\n[[preset]]\nname = \"Cli\"\nmask = \"*.h\"\n"),
    )
    .expect("write");
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let prompt = crate::tests_folder_ops::top_title(&app);
    assert_eq!(prompt.as_deref(), Some("Settings Changed on Disk"));
    let labels: Vec<String> = {
        let w = app.win();
        let p = app
            .0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<crate::dialog::Prompt>())
            .expect("the prompt");
        p.buttons.iter().map(|(_, l)| l.clone()).collect()
    };
    assert_eq!(labels, ["Overwrite", "Reload", "Cancel"]);
    // Reload: the file read again, the preset added to it.
    app.0.dialog_button(w, ButtonRole::Other(2));
    world.settle(&mut app);
    let text = std::fs::read_to_string(world.path("config/filters.toml")).expect("filters.toml");
    for name in ["Mine", "Cli", "Rust"] {
        assert!(text.contains(&format!("name = \"{name}\"")), "{text}");
    }
    assert!(text.contains("future_key"), "unknown keys kept");
}

// ---- After the W9c review: F17's nits and the Filters edit's guards (F6: M18, M19, M20) ---------------------------

/// The view's own filter entered again, spelled otherwise: nothing runs, nothing is recorded, the field shows the
/// canonical text (note 13).
#[test]
fn the_view_s_own_filter_entered_again_runs_nothing() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let w = app.win();
    folder_model(&mut app.0, view).filters_edit().text = "*.rs".into();
    app.0.filters_enter(w, None);
    world.settle(&mut app);
    let model = std::sync::Arc::clone(
        folder_model(&mut app.0, view)
            .prepared()
            .expect("p")
            .model(),
    );
    let recent = app.0.filters_choices().1.len();
    folder_model(&mut app.0, view).filters_edit().text = " *.rs ".into();
    app.0.filters_enter(w, None);
    world.settle(&mut app);
    let m = folder_model(&mut app.0, view);
    assert!(
        std::sync::Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "no re-run"
    );
    assert_eq!(m.filters_edit().text, "*.rs");
    assert_eq!(app.0.filters_choices().1.len(), recent, "nothing recorded");
}

/// While the view is busy, Enter keeps the text as typed and records nothing (review F17); Add To Presets takes the
/// canonical text and leaves the field as typed.
#[test]
fn a_busy_view_keeps_the_typed_filter_and_presets_leave_the_field() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let w = app.win();
    // A drop's prompt holds the view.
    let source = world.write("work/src/x.txt", "x\n");
    app.0.drop_paths(
        w,
        crate::dnd::DropTarget::PaneTree {
            side: hikasa_core::Side::Right,
            folder: None,
        },
        vec![source],
    );
    world.settle(&mut app);
    assert!(!folder_model(&mut app.0, view).idle());
    folder_model(&mut app.0, view).filters_edit().text = "-*.txt;*.rs".into();
    app.0.filters_enter(w, None);
    world.settle(&mut app);
    assert_eq!(
        folder_model(&mut app.0, view).filters_edit().text,
        "-*.txt;*.rs"
    );
    assert!(app.0.filters_choices().1.is_empty(), "nothing recorded");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Cancel);
    world.settle(&mut app);
    // Add To Presets: the canonical text is saved; the field keeps what was typed.
    app.0.filters_add_to_presets(w);
    assert_eq!(preset_dialog(&mut app).mask, "*.rs;-*.txt");
    assert_eq!(
        folder_model(&mut app.0, view).filters_edit().text,
        "-*.txt;*.rs"
    );
}

/// A new model keeps the text being typed, also when the view had no filter (review F17).
#[test]
fn a_new_model_keeps_the_text_being_typed() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert_eq!(folder_model(&mut app.0, view).filters_edit().committed, "");
    folder_model(&mut app.0, view).filters_edit().text = "*.r".into();
    // The comparison runs again in the view (Full Refresh): a new model.
    let model = std::sync::Arc::clone(
        folder_model(&mut app.0, view)
            .prepared()
            .expect("p")
            .model(),
    );
    app.execute(CommandId::FullRefresh);
    world.settle(&mut app);
    let m = folder_model(&mut app.0, view);
    assert!(
        !std::sync::Arc::ptr_eq(m.prepared().expect("p").model(), &model),
        "a new model"
    );
    assert_eq!(m.filters_edit().text, "*.r", "the typed text stays");
    assert_eq!(m.filters_edit().committed, "");
}

/// A preset's name is taken trimmed: one that is a used name with spaces around it is refused.
#[test]
fn a_preset_name_is_trimmed() {
    let world = World::with_files(&[("filters.toml", FILTERS_FILE)]);
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    folder_model(&mut app.0, view).filters_edit().text = "*.rs".into();
    let w = app.win();
    app.0.filters_add_to_presets(w);
    preset_dialog(&mut app).name = "  Mine ".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    assert!(preset_dialog(&mut app).error.is_some(), "a used name");
    preset_dialog(&mut app).name = "  Rust ".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let text = std::fs::read_to_string(world.path("config/filters.toml")).expect("filters.toml");
    assert!(text.contains("name = \"Rust\""), "{text}");
}

/// A matrix cell changed while a preset is chosen starts from that preset's matrix (note 11; review F6, M16).
#[test]
fn a_cell_changed_from_a_preset_keeps_the_preset_s_other_cells() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let options = hikasa_folder::sync::SyncOptions {
        preset: hikasa_folder::sync::SyncPreset::MirrorRight,
        ..hikasa_folder::sync::SyncOptions::default()
    };
    let preset = options.effective_matrix();
    assert_ne!(
        preset, options.matrix,
        "the stored custom matrix differs from the preset's"
    );
    let _view = crate::tests_sync_view::open_sync(&world, &mut app.0, options);
    settings(&world, &mut app, CommandId::Rules);
    let t = tab::<SyncTab>(&mut app);
    t.set_cell(Situation::AddedLeft, SyncAction::Leave);
    assert_eq!(
        t.fields.options.preset,
        hikasa_folder::sync::SyncPreset::Custom
    );
    let mut expected = preset;
    expected.set(Situation::AddedLeft, SyncAction::Leave);
    assert_eq!(t.fields.options.matrix, expected);
}
