//! Sessions, workspaces and settings packages (M2 W2; contract §6.2 A2.1–A2.3, A2.5, A2.6): session TOML goldens
//! per type and round trips, names and collisions, auto-save rotation with injected dates, workspace goldens and
//! restore, package export/import round trips, plan goldens and refusals.

mod common;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use common::{assert_golden, fixtures, service};
use hikasa_core::WarningCode;
use hikasa_session::api::{ApiError, PlanDto};
use hikasa_session::package::{
    Category, ExportRequest, ImportChoices, MAX_ENTRIES, MAX_ENTRY_BYTES, PlanAction, export,
    plan_import, plan_restore,
};
use hikasa_session::paths::ConfigLocation;
use hikasa_session::sessions::{
    AutoGroup, AutoId, Override, OverrideScope, ParentLink, Session, SessionRef, SessionRules,
    SessionStore, SessionType,
};
use hikasa_session::settings::{LoadMode, Options, Overwrite};
use hikasa_session::workspaces::{
    RestoredTab, Workspace, WorkspaceStore, WorkspaceTab, WorkspaceWindow, inline_table, restore,
};
use proptest::prelude::*;
use toml::{Table, Value};

/// A configuration at `root/config` (the service's).
fn config(root: &Path) -> ConfigLocation {
    service(root).config().clone()
}

/// A store with `max_auto_saved_sessions = max`.
fn store(root: &Path, max: u32) -> SessionStore {
    let mut options = Options::default();
    options.tweaks.metrics.max_auto_saved_sessions = max;
    SessionStore::new(&config(root), &options)
}

fn r(text: &str) -> SessionRef {
    SessionRef::parse(text).unwrap()
}

fn table(text: &str) -> Table {
    toml::from_str(text).unwrap()
}

/// A session of `kind` that differs from the defaults in every part.
fn sample(kind: SessionType) -> Session {
    let mut s = Session::builtin(kind);
    s.description = format!("A {kind} session");
    s.favorite = true;
    s.specs.left = "/data/left".into();
    if kind != SessionType::TextEdit {
        s.specs.right = "/data/right".into();
    }
    match &mut s.rules {
        SessionRules::Folder(rules) => {
            rules.handling.flatten = true;
            rules.criteria.size = false;
        }
        SessionRules::Text(rules) => {
            rules.alignment.skew_tolerance = 7;
            s.specs.format = "c-cpp".into();
            s.specs.left_encoding = "utf-8".into();
        }
        SessionRules::Opaque(t) => {
            *t = table("mode = \"mirror\"\n[filters]\nnames = \"*.c\"\n");
        }
    }
    if let Some(text) = &mut s.text_rules {
        text.importance.compare_line_endings = true;
    }
    s.display = table("filter = \"differences\"\n");
    s.view = table("split = 0.5\nwidth_left = 300\n");
    if kind.is_folder() {
        s.overrides.push(Override {
            scope: OverrideScope::Pair,
            left: "src/a.c".into(),
            right: "src/a.c".into(),
            kind: SessionType::TextCompare,
            rules: table("[alignment]\nskew_tolerance = 3\n"),
        });
    }
    if kind == SessionType::TextCompare {
        s.parent = Some(ParentLink {
            session: "work/folders".into(),
            left: "src/a.c".into(),
            right: "src/a.c".into(),
        });
    }
    s
}

/// A2.1: one TOML golden per type; load gives the session back.
#[test]
fn session_goldens_per_type() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 200);
    for kind in SessionType::ALL {
        let session = sample(kind);
        let reference = r(&format!("golden/{kind}"));
        let path = store
            .save(&reference, &session, Overwrite::IfUnchanged)
            .unwrap();
        assert_golden(
            &fixtures("sessions").join(format!("{kind}.toml")),
            &std::fs::read(&path).unwrap(),
        );
        let loaded = store.load(&reference, LoadMode::Strict).unwrap();
        assert_eq!(loaded.value, session, "{kind}");
    }
}

/// A2.1: P1 type names are refused (a file and `SessionType::parse`).
#[test]
fn p1_types_are_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 200);
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(store.dir().join("m.toml"), "type = \"text-merge\"\n").unwrap();
    let err = store.load(&r("m"), LoadMode::Strict).unwrap_err();
    assert!(matches!(err, ApiError::Unsupported(_)), "{err:?}");
}

/// A2.1: `-` and `clipboard://` are never stored; other specs are stored canonical and absolute.
#[test]
fn stdin_and_clipboard_are_not_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let s = service(tmp.path());
    for text in ["-", "clipboard://"] {
        let (spec, warning) = s.stored_spec(text).unwrap();
        assert_eq!(spec, "");
        assert_eq!(warning.unwrap().code, WarningCode::SpecNotSaved);
    }
    let (spec, warning) = s.stored_spec("a.txt").unwrap();
    assert!(warning.is_none());
    assert!(
        Path::new(&spec).is_absolute() && spec.ends_with("a.txt"),
        "{spec}"
    );
    let dto = s
        .session_new(
            &r("x"),
            SessionType::TextCompare,
            Some("-"),
            Some("b.txt"),
            None,
        )
        .unwrap();
    assert_eq!(dto.warnings.len(), 1);
    let text = std::fs::read_to_string(&dto.written[0]).unwrap();
    assert!(!text.contains("left ="), "{text}");
    assert!(text.contains("b.txt"), "{text}");
}

/// A2.1: rules are written sparse against the defaults at save time; a later change of the defaults reaches the
/// session, which keeps its own differences.
#[test]
fn sparse_rules_follow_the_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 200);
    let kind = SessionType::TextCompare;
    let mut defaults = Session::builtin(kind);
    let SessionRules::Text(rules) = &mut defaults.rules else {
        panic!("text rules")
    };
    rules.alignment.skew_tolerance = 5;
    store.save_defaults(&defaults).unwrap();
    let mut session = Session::new(kind, &store.defaults(kind).unwrap());
    let SessionRules::Text(rules) = &mut session.rules else {
        panic!("text rules")
    };
    rules.importance.compare_line_endings = true;
    let path = store
        .save(&r("own"), &session, Overwrite::IfUnchanged)
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("compare_line_endings = true"), "{text}");
    assert!(!text.contains("skew_tolerance"), "{text}");
    // The defaults change: the session sees the new skew and keeps its own difference.
    let SessionRules::Text(rules) = &mut defaults.rules else {
        panic!("text rules")
    };
    rules.alignment.skew_tolerance = 9;
    store.save_defaults(&defaults).unwrap();
    let loaded = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    let SessionRules::Text(rules) = loaded.rules else {
        panic!("text rules")
    };
    assert_eq!(rules.alignment.skew_tolerance, 9);
    assert!(rules.importance.compare_line_endings);
}

fn arb_session(kind: SessionType) -> impl Strategy<Value = Session> {
    (
        "[a-zA-Z0-9 ]{0,12}",
        any::<bool>(),
        any::<bool>(),
        "(/[a-z]{1,8}){1,3}",
        "(/[a-z]{1,8}){0,3}",
        any::<bool>(),
        0u32..100,
        prop::collection::btree_map("[a-z]{1,6}", 0i64..1000, 0..3),
    )
        .prop_map(move |(d, locked, fav, left, right, flag, n, view)| {
            let mut s = Session::builtin(kind);
            s.description = d;
            s.locked = locked;
            s.favorite = fav;
            s.specs.left = left;
            if kind != SessionType::TextEdit {
                s.specs.right = right;
            }
            match &mut s.rules {
                SessionRules::Folder(rules) => {
                    rules.handling.flatten = flag;
                    rules.criteria.timestamps.tolerance_secs = n;
                }
                SessionRules::Text(rules) => {
                    rules.importance.compare_line_endings = flag;
                    rules.alignment.skew_tolerance = n;
                }
                SessionRules::Opaque(t) => {
                    t.insert("n".into(), Value::Integer(i64::from(n)));
                }
            }
            s.view = view
                .into_iter()
                .map(|(k, v)| (k, Value::Integer(v)))
                .collect();
            s
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// A2.1: save → load gives the session back, for every type.
    #[test]
    fn sessions_round_trip(
        sessions in (
            arb_session(SessionType::FolderCompare),
            arb_session(SessionType::FolderSync),
            arb_session(SessionType::TextCompare),
            arb_session(SessionType::TextEdit),
            arb_session(SessionType::TableCompare),
            arb_session(SessionType::HexCompare),
            arb_session(SessionType::PictureCompare),
        )
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path(), 200);
        let all = [sessions.0, sessions.1, sessions.2, sessions.3, sessions.4, sessions.5, sessions.6];
        for (k, session) in all.iter().enumerate() {
            let reference = r(&format!("s{k}"));
            store.save(&reference, session, Overwrite::Force).unwrap();
            let loaded = store.load(&reference, LoadMode::Strict).unwrap().value;
            prop_assert_eq!(&loaded, session);
            // Saving again writes nothing new.
            let before = std::fs::read(store.path(&reference)).unwrap();
            store.save(&reference, &loaded, Overwrite::Force).unwrap();
            prop_assert_eq!(before, std::fs::read(store.path(&reference)).unwrap());
        }
    }
}

/// A2.2: case-insensitive collisions on save and rename, folder moves, locked files, folder deletion.
#[test]
fn names_collide_case_insensitively() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 200);
    let s = Session::builtin(SessionType::TextCompare);
    store
        .save(&r("Work/Api"), &s, Overwrite::IfUnchanged)
        .unwrap();
    // Another case of the same name: refused; the folder keeps its spelling.
    let err = store
        .save(&r("work/API"), &s, Overwrite::IfUnchanged)
        .unwrap_err();
    assert!(matches!(err, ApiError::AlreadyExists(_)), "{err:?}");
    let path = store
        .save(&r("work/other"), &s, Overwrite::IfUnchanged)
        .unwrap();
    assert!(path.ends_with("Work/other.toml"), "{}", path.display());
    assert_eq!(store.find("work/api"), Some(r("Work/Api")));
    assert_eq!(store.find("Work/Api"), Some(r("Work/Api")));
    assert_eq!(store.find("nope"), None);
    // Rename onto an existing session: refused; a change of case only: allowed.
    let err = store.rename(&r("Work/Api"), &r("work/OTHER")).unwrap_err();
    assert!(matches!(err, ApiError::AlreadyExists(_)), "{err:?}");
    store.rename(&r("Work/Api"), &r("Work/api")).unwrap();
    assert_eq!(store.find("work/API"), Some(r("Work/api")));
    // A move into new folders.
    store
        .rename(&r("Work/api"), &r("Archive/2026/api"))
        .unwrap();
    assert!(store.path(&r("Archive/2026/api")).is_file());
    // A locked session is not overwritten unless forced.
    store.set_locked(&r("Archive/2026/api"), true).unwrap();
    let err = store
        .save(&r("Archive/2026/api"), &s, Overwrite::IfUnchanged)
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
    store
        .save(&r("Archive/2026/api"), &s, Overwrite::Force)
        .unwrap();
    // Folders: a non-empty folder needs `recursive`.
    let archive = [r("Archive").name];
    assert!(matches!(
        store.delete_folder(&archive, false),
        Err(ApiError::InvalidArgument(_))
    ));
    store.rename_folder(&archive, &[r("Old").name]).unwrap();
    store.delete_folder(&[r("Old").name], true).unwrap();
    assert_eq!(store.find("Old/2026/api"), None);
    let err = store.create_folder(&[r("WORK").name]).unwrap_err();
    assert!(matches!(err, ApiError::AlreadyExists(_)), "{err:?}");
}

/// The time `days` days after 2026-10-21 12:00 UTC (+ `secs`).
fn at(days: i64, secs: i64) -> jiff::Zoned {
    let ts = jiff::Timestamp::from_second(1_792_584_000 + days * 86_400 + secs).unwrap();
    ts.to_zoned(jiff::tz::TimeZone::UTC)
}

/// A2.3: rotation with injected dates: ids, slot reuse, groups, pruning order, 0 disables and clears, named
/// sessions untouched.
#[test]
fn autosave_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Session::builtin(SessionType::TextCompare);
    let keep3 = store(tmp.path(), 3);
    keep3.save(&r("named"), &s, Overwrite::IfUnchanged).unwrap();
    let first = keep3.autosave(None, &s, &at(0, 0)).unwrap().unwrap();
    assert_eq!(first.as_str(), "20261021-120000-1");
    // The same second again: the next free counter.
    let second = keep3.autosave(None, &s, &at(0, 0)).unwrap().unwrap();
    assert_eq!(second.as_str(), "20261021-120000-2");
    // A later save of the same view overwrites its slot.
    let again = keep3
        .autosave(Some(&first), &s, &at(1, 0))
        .unwrap()
        .unwrap();
    assert_eq!(again, first);
    let text = std::fs::read_to_string(keep3.autosaved_path(&first)).unwrap();
    assert!(
        text.contains("saved_at = \"2026-10-22T12:00:00+00:00\""),
        "{text}"
    );
    // Two more: the oldest by saved_at (`second`, still on day 0) is pruned first.
    keep3.autosave(None, &s, &at(2, 0)).unwrap().unwrap();
    let fourth = keep3.autosave(None, &s, &at(3, 0)).unwrap().unwrap();
    let ids: Vec<String> = keep3
        .list(&at(3, 0))
        .unwrap()
        .autosaved
        .iter()
        .map(|e| e.id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            fourth.to_string(),
            "20261023-120000-1".into(),
            first.to_string()
        ]
    );
    // Groups against the injected date: today, yesterday, 2…6, 7 days, future.
    let tmp2 = tempfile::tempdir().unwrap();
    let many = store(tmp2.path(), 100);
    for (days, _) in [(0, ""), (-1, ""), (-2, ""), (-6, ""), (-7, ""), (3, "")] {
        many.autosave(None, &s, &at(days, 0)).unwrap();
    }
    let groups: BTreeMap<String, AutoGroup> = many
        .list(&at(0, 3600))
        .unwrap()
        .autosaved
        .into_iter()
        .map(|e| (e.id.to_string(), e.group))
        .collect();
    let want = [
        ("20261021-120000-1", AutoGroup::Today),
        ("20261020-120000-1", AutoGroup::Yesterday),
        ("20261019-120000-1", AutoGroup::Days2To6),
        ("20261015-120000-1", AutoGroup::Days2To6),
        ("20261014-120000-1", AutoGroup::Older),
        ("20261024-120000-1", AutoGroup::Today),
    ];
    for (id, group) in want {
        assert_eq!(groups.get(id), Some(&group), "{id}");
    }
    // 0: nothing is saved and every auto-saved file goes; named sessions stay.
    let off = store(tmp.path(), 0);
    assert_eq!(off.autosave(None, &s, &at(4, 0)).unwrap(), None);
    assert!(off.list(&at(4, 0)).unwrap().autosaved.is_empty());
    assert!(off.path(&r("named")).is_file());
    assert_eq!(keep3.prune_autosaved(5).unwrap(), Vec::<AutoId>::new());
}

/// A2.5: a workspace golden, round trip, and a missing session → Home tab + `session_missing`.
#[test]
fn workspace_golden_and_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = store(tmp.path(), 200);
    let workspaces = WorkspaceStore::new(&config(tmp.path()));
    let s = sample(SessionType::FolderCompare);
    sessions
        .save(&r("work/api"), &s, Overwrite::IfUnchanged)
        .unwrap();
    let mut inline = sample(SessionType::TextCompare);
    inline.parent = None;
    let workspace = Workspace {
        shortcut: "Ctrl+Alt+1".into(),
        windows: vec![
            WorkspaceWindow {
                active_tab: 1,
                tabs: vec![
                    WorkspaceTab {
                        session: "work/api".into(),
                        view: table("scroll = 10\n"),
                        ..WorkspaceTab::default()
                    },
                    WorkspaceTab {
                        inline: inline_table(&inline, &sessions, None).unwrap(),
                        ..WorkspaceTab::default()
                    },
                    WorkspaceTab {
                        session: "gone".into(),
                        ..WorkspaceTab::default()
                    },
                ],
                ..WorkspaceWindow::default()
            },
            WorkspaceWindow {
                width: 640,
                height: 480,
                x: 10,
                y: 20,
                maximized: true,
                ..WorkspaceWindow::default()
            },
        ],
    };
    let name = hikasa_session::sessions::SessionName::parse("last").unwrap();
    let path = workspaces.save(&name, &workspace).unwrap();
    assert_golden(
        &fixtures("sessions").join("workspace.toml"),
        &std::fs::read(&path).unwrap(),
    );
    let loaded = workspaces.load(&name, LoadMode::Strict).unwrap();
    assert_eq!(loaded.value, workspace);
    let (windows, warnings) = restore(&loaded.value, &sessions);
    assert_eq!(windows.len(), 2);
    let tabs = &windows[0].tabs;
    assert!(matches!(&tabs[0], RestoredTab::Named { session, .. } if **session == s));
    assert!(matches!(&tabs[1], RestoredTab::Inline(session) if **session == inline));
    assert_eq!(tabs[2], RestoredTab::Home);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, WarningCode::SessionMissing);
    // A tab with both contents is a problem; lenient loading keeps it as it is.
    std::fs::write(&path, "[[window]]\nactive_tab = 9\n[[window.tab]]\n").unwrap();
    assert!(workspaces.load(&name, LoadMode::Strict).is_err());
    let lenient = workspaces.load(&name, LoadMode::Lenient).unwrap();
    assert_eq!(lenient.value.windows[0].active_tab, 0);
}

/// A configuration with a file in every category.
fn populate(root: &Path) -> hikasa_session::Service {
    let svc = service(root);
    for (key, value) in [
        ("options.appearance.theme_mode", "dark"),
        ("theme.fonts.editor.size", "15.0"),
        ("state.window.width", "1000"),
        ("filters.preset", "[{name = \"C\", mask = \"*.c\"}]"),
    ] {
        let key = hikasa_session::settings::parse_key(key).unwrap();
        svc.settings_set(&key, value, false).unwrap();
    }
    let dir = &svc.config().dir;
    std::fs::create_dir_all(dir.join("themes")).unwrap();
    std::fs::write(
        dir.join("themes/mine.toml"),
        "schema_version = 1\nbase = \"dark\"\n",
    )
    .unwrap();
    let store = svc.session_store();
    store
        .save(
            &r("work/api"),
            &sample(SessionType::FolderCompare),
            Overwrite::IfUnchanged,
        )
        .unwrap();
    store
        .save(
            &r("notes"),
            &sample(SessionType::TextEdit),
            Overwrite::IfUnchanged,
        )
        .unwrap();
    store
        .save_defaults(&sample(SessionType::HexCompare))
        .unwrap();
    store
        .autosave(None, &Session::builtin(SessionType::TextCompare), &at(0, 0))
        .unwrap();
    svc.format_new("my-c", Some("c-cpp"), None, None, None)
        .unwrap();
    let ws = hikasa_session::sessions::SessionName::parse("last").unwrap();
    WorkspaceStore::new(svc.config())
        .save(
            &ws,
            &Workspace {
                shortcut: "F9".into(),
                ..Workspace::default()
            },
        )
        .unwrap();
    svc
}

/// Every file below `dir` (relative path → bytes), `_autosaved` excluded.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("_autosaved") {
                    stack.push(p);
                }
            } else {
                let rel = p
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// A2.6: export → import into an empty configuration reproduces every file byte for byte; two exports at the
/// same instant are identical; `system_info` is exported only when asked and never imported.
#[test]
fn package_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = populate(tmp.path());
    let out1 = tmp.path().join("a.hikasa-pkg");
    let out2 = tmp.path().join("b.hikasa-pkg");
    let dto = svc
        .settings_export(&ExportRequest::default(), &out1)
        .unwrap();
    svc.settings_export(&ExportRequest::default(), &out2)
        .unwrap();
    assert_eq!(std::fs::read(&out1).unwrap(), std::fs::read(&out2).unwrap());
    let paths: Vec<&str> = dto.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "filters.toml",
            "formats/my-c.toml",
            "formats/order.toml",
            "options.toml",
            "sessions/_defaults/hex-compare.toml",
            "sessions/notes.toml",
            "sessions/work/api.toml",
            "state.toml",
            "theme.toml",
            "themes/mine.toml",
            "workspaces/last.toml",
        ]
    );
    let other = tempfile::tempdir().unwrap();
    let target = service(other.path());
    let result = target
        .settings_import(&out1, &ImportChoices::default(), false)
        .unwrap();
    assert!(result.ok(), "{result:?}");
    assert_eq!(files(&target.config().dir), files(&svc.config().dir));
    // System info on request: no user name, the home folder as `~`.
    let info = ExportRequest {
        categories: vec![Category::SystemInfo],
        ..ExportRequest::default()
    };
    let out3 = tmp.path().join("c.hikasa-pkg");
    let now = svc.clock().now();
    let dto = export(svc.config(), &info, &now, svc.env(), &out3).unwrap();
    assert_eq!(dto.items.len(), 1);
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&out3).unwrap()).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("system-info.txt").unwrap(), &mut text)
        .unwrap();
    assert!(!text.contains("tester"), "{text}");
    assert!(text.contains("Hikasa "), "{text}");
    let err = plan_import(
        target.config(),
        &out3,
        &ImportChoices {
            categories: vec![Category::SystemInfo],
            ..ImportChoices::default()
        },
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
}

/// JSON of a plan with the temporary root replaced by `<TMP>`.
fn plan_json(plan: &hikasa_session::package::ImportPlan, root: &Path) -> Vec<u8> {
    let text = serde_json::to_string_pretty(&PlanDto::of(plan)).unwrap();
    let root = root.display().to_string();
    let mut text = text.replace(&root, "<TMP>").replace('\\', "/");
    text.push('\n');
    text.into_bytes()
}

/// A2.6: plan goldens: collisions, deletes and the summary; a restore plan.
#[test]
fn import_and_restore_plan_goldens() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = populate(tmp.path());
    let package = tmp.path().join("p.hikasa-pkg");
    svc.settings_export(&ExportRequest::default(), &package)
        .unwrap();
    // Into a configuration that holds some of the items and others.
    let other = tempfile::tempdir().unwrap();
    let target = service(other.path());
    let tstore = target.session_store();
    tstore
        .save(
            &r("work/api"),
            &Session::builtin(SessionType::FolderCompare),
            Overwrite::IfUnchanged,
        )
        .unwrap();
    tstore
        .save(
            &r("mine"),
            &Session::builtin(SessionType::TextCompare),
            Overwrite::IfUnchanged,
        )
        .unwrap();
    let key = hikasa_session::settings::parse_key("options.appearance.theme_mode").unwrap();
    target.settings_set(&key, "light", false).unwrap();
    let plan = plan_import(
        target.config(),
        &package,
        &ImportChoices {
            categories: vec![Category::Options, Category::Sessions],
            items: BTreeMap::from([(Category::Formats, vec!["my-c".to_owned()])]),
            delete_existing: [Category::Sessions].into_iter().collect(),
            force: false,
        },
    )
    .unwrap();
    assert_golden(
        &fixtures("sessions").join("import-plan.json"),
        &plan_json(&plan, other.path()),
    );
    let plan = plan_import(target.config(), &package, &ImportChoices::default()).unwrap();
    assert_golden(
        &fixtures("sessions").join("import-plan-all.json"),
        &plan_json(&plan, other.path()),
    );
    let restore = plan_restore(
        svc.config(),
        &[Category::Options, Category::Sessions, Category::Formats],
        false,
        true,
        false,
    );
    assert_golden(
        &fixtures("sessions").join("restore-plan.json"),
        &plan_json(&restore, tmp.path()),
    );
    // Restoring with the explicit choice also deletes the named sessions.
    let all = plan_restore(svc.config(), &[], true, false, false);
    assert_eq!(all.deletes.len(), 3, "{:?}", all.deletes);
}

/// Writes a zip of `entries` (stored) with a manifest listing `listed` (`(category, name, path)`).
fn package(path: &Path, entries: &[(&str, &[u8])], listed: &[(&str, &str, &str)]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut manifest = String::from(
        "schema_version = 1\nformat = \"hikasa-pkg\"\nhikasa_version = \"0.2.0\"\ncreated = \"2026-10-28T12:00:00+01:00\"\n",
    );
    for (c, n, p) in listed {
        manifest.push_str(&format!(
            "[[item]]\ncategory = \"{c}\"\nname = \"{n}\"\npath = \"{p}\"\n"
        ));
    }
    zip.start_file("manifest.toml", options).unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

/// A refusal case: name, entries, manifest items, expected error kind.
type Case<'a> = (
    &'a str,
    Vec<(&'a str, &'a [u8])>,
    Vec<(&'a str, &'a str, &'a str)>,
    &'a str,
);

/// A2.6: refusals; in each case nothing is written.
#[test]
fn import_refusals_write_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let opts: &[u8] = b"schema_version = 1\n";
    let big = vec![b'#'; usize::try_from(MAX_ENTRY_BYTES).unwrap() + 1];
    let cases: Vec<Case<'_>> = vec![
        (
            "traversal",
            vec![("../options.toml", opts)],
            vec![],
            "invalid_data",
        ),
        (
            "absolute",
            vec![("/options.toml", opts)],
            vec![],
            "invalid_data",
        ),
        (
            "backslash",
            vec![("sessions\\a.toml", opts)],
            vec![],
            "invalid_data",
        ),
        (
            "dots",
            vec![("sessions/./a.toml", opts)],
            vec![],
            "invalid_data",
        ),
        (
            "too-large",
            vec![("options.toml", &big)],
            vec![("options", "options", "options.toml")],
            "invalid_data",
        ),
        (
            "unlisted",
            vec![("options.toml", opts)],
            vec![],
            "invalid_data",
        ),
        (
            "missing-entry",
            vec![],
            vec![("options", "options", "options.toml")],
            "invalid_data",
        ),
        (
            "wrong-category",
            vec![("options.toml", opts)],
            vec![("state", "options", "options.toml")],
            "invalid_data",
        ),
        (
            "not-allowed",
            vec![("evil.toml", opts)],
            vec![("options", "evil", "evil.toml")],
            "invalid_data",
        ),
        (
            "newer-schema",
            vec![("options.toml", b"schema_version = 99\n")],
            vec![("options", "options", "options.toml")],
            "unsupported",
        ),
        (
            "bad-session",
            vec![("sessions/a.toml", b"type = \"nope\"\n")],
            vec![("sessions", "a", "sessions/a.toml")],
            "invalid_data",
        ),
        (
            "not-utf8",
            vec![("state.toml", b"\xff\xfe")],
            vec![("state", "state", "state.toml")],
            "invalid_data",
        ),
    ];
    for (name, entries, listed, kind) in cases {
        let path = tmp.path().join(format!("{name}.hikasa-pkg"));
        package(&path, &entries, &listed);
        let err = svc
            .settings_import(&path, &ImportChoices::default(), false)
            .unwrap_err();
        assert_eq!(err.kind().as_str(), kind, "{name}: {err}");
        assert!(!svc.config().dir.exists(), "{name}: something was written");
    }
    // A newer manifest, and not a zip at all.
    let newer = tmp.path().join("newer.hikasa-pkg");
    package(&newer, &[], &[]);
    let bytes = std::fs::read(&newer).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&newer).unwrap());
    zip.start_file("manifest.toml", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        b"schema_version = 7\nformat = \"hikasa-pkg\"\nhikasa_version = \"9\"\ncreated = \"\"\n",
    )
    .unwrap();
    zip.finish().unwrap();
    assert_ne!(bytes, std::fs::read(&newer).unwrap());
    let err = svc
        .settings_import(&newer, &ImportChoices::default(), true)
        .unwrap_err();
    assert_eq!(err.kind().as_str(), "unsupported", "{err}");
    let junk = tmp.path().join("junk.hikasa-pkg");
    std::fs::write(&junk, b"not a zip").unwrap();
    let err = svc
        .settings_import(&junk, &ImportChoices::default(), true)
        .unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(!svc.config().dir.exists());
}

/// A2.6: more entries than the cap are refused before anything is read.
#[test]
fn too_many_entries_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let path = tmp.path().join("many.hikasa-pkg");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..=MAX_ENTRIES {
        zip.start_file(format!("d/{i}"), options).unwrap();
    }
    zip.finish().unwrap();
    let err = svc
        .settings_import(&path, &ImportChoices::default(), false)
        .unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(err.to_string().contains("entries"), "{err}");
    assert!(!svc.config().dir.exists());
}

/// Imports `entries`/`listed` into a fresh configuration and returns the error (nothing may be written).
fn refused(name: &str, entries: &[(&str, &[u8])], listed: &[(&str, &str, &str)]) -> ApiError {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let path = tmp.path().join(format!("{name}.hikasa-pkg"));
    package(&path, entries, listed);
    let err = svc
        .settings_import(&path, &ImportChoices::default(), false)
        .unwrap_err();
    assert!(!svc.config().dir.exists(), "{name}: something was written");
    err
}

/// Review W2 #1: import validates strictly (as the commands that later read the files): type errors, problems,
/// rules that do not compile and bad workspace tabs refuse the whole package, naming the entry and the key; a
/// package whose last item is bad writes nothing (A2.6).
#[test]
fn import_validates_strictly_and_writes_nothing() {
    let ok: &[u8] = b"type = \"text-compare\"\n[specs]\nleft = \"/a\"\nright = \"/b\"\n";
    let cases: &[(&str, &str, &[u8], &str)] = &[
        (
            "options.toml",
            "options",
            b"schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = \"x\"\n",
            "max_auto_saved_sessions",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"text-compare\"\n[specs]\nleft = 5\n",
            "left",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"folder-compare\"\n[[text_rules.importance.unimportant]]\npattern = \"(\"\nregex = true\n",
            "text_rules",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"text-compare\"\n[[rules.importance.unimportant]]\npattern = \"(\"\nregex = true\n",
            "rules",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"folder-compare\"\n[rules.criteria.timestamps.offset]\nside = \"left\"\nhours = 40\n",
            "rules",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"folder-compare\"\n[rules]\nfilters = 7\n",
            "rules",
        ),
        (
            "sessions/bad.toml",
            "bad",
            b"type = \"folder-compare\"\n[[override]]\nscope = \"type\"\ntype = \"text-compare\"\n[[override.rules.importance.unimportant]]\npattern = \"(\"\nregex = true\n",
            "override.0.rules",
        ),
        (
            "workspaces/w.toml",
            "w",
            b"[[window]]\n[[window.tab]]\nsession = \"x\"\n[window.tab.inline]\ntype = \"text-compare\"\n",
            "window.0.tab.0",
        ),
        (
            "workspaces/w.toml",
            "w",
            b"[[window]]\n[[window.tab]]\n[window.tab.inline]\ntype = \"nope\"\n",
            "window.0.tab.0.inline",
        ),
    ];
    for (path, name, bytes, key) in cases {
        let category = if path.starts_with("sessions") {
            "sessions"
        } else if path.starts_with("workspaces") {
            "workspaces"
        } else {
            "options"
        };
        // The bad entry is the last item of the manifest.
        let err = refused(
            name,
            &[("sessions/aaa.toml", ok), (path, bytes)],
            &[
                ("sessions", "aaa", "sessions/aaa.toml"),
                (category, name, path),
            ],
        );
        let text = err.to_string();
        assert_eq!(err.kind().as_str(), "invalid_data", "{path}: {text}");
        assert!(text.contains(path), "{path}: {text}");
        assert!(text.contains(key), "{path}: {text}");
        // Review W2 #11: the entry is named once.
        assert_eq!(text.matches(path).count(), 1, "{text}");
    }
}

/// Review W2 #11: an unknown category in the manifest is `invalid_data` (not an argument error).
#[test]
fn unknown_manifest_category_is_invalid_data() {
    let opts: &[u8] = b"schema_version = 1\n";
    let err = refused(
        "bogus",
        &[("options.toml", opts)],
        &[("bogus", "options", "options.toml")],
    );
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(err.to_string().contains("bogus"), "{err}");
}

/// Review W2 #2: names are unique whatever their case (D-80): twins inside a package are refused; an item that
/// matches an existing file or folder in another case replaces it, keeping its spelling.
#[test]
fn import_collisions_ignore_case() {
    let ses: &[u8] = b"type = \"text-compare\"\ndescription = \"from the package\"\n";
    let err = refused(
        "twins",
        &[("sessions/A.toml", ses), ("sessions/a.toml", ses)],
        &[
            ("sessions", "A", "sessions/A.toml"),
            ("sessions", "a", "sessions/a.toml"),
        ],
    );
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(err.to_string().contains("case"), "{err}");
    let ws: &[u8] = b"shortcut = \"F2\"\n";
    let err = refused(
        "ws-twins",
        &[("workspaces/W.toml", ws), ("workspaces/w.toml", ws)],
        &[
            ("workspaces", "W", "workspaces/W.toml"),
            ("workspaces", "w", "workspaces/w.toml"),
        ],
    );
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");

    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let store = svc.session_store();
    let s = Session::builtin(SessionType::TextCompare);
    store.save(&r("Top"), &s, Overwrite::IfUnchanged).unwrap();
    store
        .save(&r("work/api"), &s, Overwrite::IfUnchanged)
        .unwrap();
    WorkspaceStore::new(svc.config())
        .save(
            &hikasa_session::sessions::SessionName::parse("Last").unwrap(),
            &Workspace {
                shortcut: "F3".into(),
                ..Workspace::default()
            },
        )
        .unwrap();
    let path = tmp.path().join("case.hikasa-pkg");
    package(
        &path,
        &[
            ("sessions/top.toml", ses),
            ("sessions/WORK/API.toml", ses),
            ("sessions/WORK/new.toml", ses),
            ("workspaces/last.toml", ws),
        ],
        &[
            ("sessions", "top", "sessions/top.toml"),
            ("sessions", "WORK/API", "sessions/WORK/API.toml"),
            ("sessions", "WORK/new", "sessions/WORK/new.toml"),
            ("workspaces", "last", "workspaces/last.toml"),
        ],
    );
    let dto = svc
        .settings_import(&path, &ImportChoices::default(), false)
        .unwrap();
    assert!(dto.ok(), "{:?}", dto.results);
    let actions: Vec<(String, String)> = dto
        .plan
        .items
        .iter()
        .map(|i| {
            let rel = Path::new(&i.path)
                .strip_prefix(&svc.config().dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (
                rel,
                serde_json::to_value(i.action)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actions,
        [
            ("sessions/Top.toml", "replace"),
            ("sessions/work/api.toml", "replace"),
            ("sessions/work/new.toml", "add"),
            ("workspaces/Last.toml", "replace"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    let dir = &svc.config().dir;
    let names = |d: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(&dir.join("sessions")), ["Top.toml", "work"]);
    assert_eq!(names(&dir.join("sessions/work")), ["api.toml", "new.toml"]);
    assert_eq!(names(&dir.join("workspaces")), ["Last.toml"]);
    let top = std::fs::read_to_string(dir.join("sessions/Top.toml")).unwrap();
    assert!(top.contains("from the package"), "{top}");
}

/// Review W2 #9: a locked named session is neither replaced nor deleted by an import unless forced
/// (`skip_locked`, warning `session_locked`).
#[test]
fn import_keeps_locked_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let store = svc.session_store();
    let s = Session::builtin(SessionType::TextCompare);
    store.save(&r("keep"), &s, Overwrite::IfUnchanged).unwrap();
    store.save(&r("gone"), &s, Overwrite::IfUnchanged).unwrap();
    store.save(&r("held"), &s, Overwrite::IfUnchanged).unwrap();
    store.set_locked(&r("keep"), true).unwrap();
    store.set_locked(&r("held"), true).unwrap();
    let before = std::fs::read(store.path(&r("keep"))).unwrap();
    let ses: &[u8] = b"type = \"text-compare\"\ndescription = \"new\"\n";
    let path = tmp.path().join("locked.hikasa-pkg");
    package(
        &path,
        &[("sessions/KEEP.toml", ses)],
        &[("sessions", "KEEP", "sessions/KEEP.toml")],
    );
    let choices = ImportChoices {
        delete_existing: [Category::Sessions].into_iter().collect(),
        ..ImportChoices::default()
    };
    let dto = svc.settings_import(&path, &choices, false).unwrap();
    assert!(dto.ok(), "{:?}", dto.results);
    assert_eq!(
        serde_json::to_value(dto.plan.items[0].action).unwrap(),
        "skip_locked"
    );
    // `keep` and `held` are not deleted, and `keep` is not replaced.
    assert_eq!(dto.warnings.len(), 3, "{:?}", dto.warnings);
    assert!(
        dto.warnings
            .iter()
            .all(|w| w.code == WarningCode::SessionLocked)
    );
    assert_eq!(std::fs::read(store.path(&r("keep"))).unwrap(), before);
    assert!(store.path(&r("held")).is_file());
    assert!(!store.path(&r("gone")).exists());
    // --force replaces and deletes locked sessions too.
    let forced = ImportChoices {
        force: true,
        ..choices
    };
    let dto = svc.settings_import(&path, &forced, false).unwrap();
    assert!(dto.ok() && dto.warnings.is_empty(), "{:?}", dto.warnings);
    assert!(!store.path(&r("held")).exists());
    let text = std::fs::read_to_string(store.path(&r("keep"))).unwrap();
    assert!(text.contains("description = \"new\""), "{text}");
}

/// D-91: `restore-defaults --delete-sessions` keeps locked named sessions (`skip_locked`, warning
/// `session_locked`) unless forced; the defaults files are deleted either way.
#[test]
fn restore_keeps_locked_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let store = svc.session_store();
    let s = Session::builtin(SessionType::TextCompare);
    store.save(&r("keep"), &s, Overwrite::IfUnchanged).unwrap();
    store.save(&r("gone"), &s, Overwrite::IfUnchanged).unwrap();
    store.set_locked(&r("keep"), true).unwrap();
    let mut defaults = Session::builtin(SessionType::TextCompare);
    defaults.description = "mine".into();
    let defaults_file = store.save_defaults(&defaults).unwrap();
    assert!(defaults_file.is_file(), "{}", defaults_file.display());
    let plan = plan_restore(svc.config(), &[Category::Sessions], true, false, false);
    assert_eq!(plan.items.len(), 1, "{:?}", plan.items);
    assert_eq!(plan.items[0].name, "keep");
    assert_eq!(plan.items[0].action, PlanAction::SkipLocked);
    assert_eq!(plan.deletes.len(), 2, "{:?}", plan.deletes);
    assert_eq!(plan.summary[&Category::Sessions].deleted, 2);
    // The dry run and the execution report the kept session as a warning only.
    let dto = svc
        .settings_restore_categories(&[Category::Sessions], true, false, false, true)
        .unwrap();
    assert_eq!(dto.files.len(), 2);
    let dto = svc
        .settings_restore_categories(&[Category::Sessions], true, false, false, false)
        .unwrap();
    assert_eq!(dto.warnings.len(), 1, "{:?}", dto.warnings);
    assert_eq!(dto.warnings[0].code, WarningCode::SessionLocked);
    assert!(dto.warnings[0].message.contains("restore with --force"));
    assert!(store.path(&r("keep")).is_file());
    assert!(!store.path(&r("gone")).exists());
    assert!(!defaults_file.exists());
    // --force deletes it too.
    let dto = svc
        .settings_restore_categories(&[Category::Sessions], true, false, true, false)
        .unwrap();
    assert!(dto.warnings.is_empty() && dto.files.len() == 1, "{dto:?}");
    assert!(!store.path(&r("keep")).exists());
}

/// A2.6: an entry whose header lies about its uncompressed size, and a package over the total cap, are refused.
#[test]
fn lying_sizes_and_the_total_cap_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    // One entry larger than the cap whose headers claim 100 bytes.
    let mut big = b"type = \"text-compare\"\n#".to_vec();
    big.resize(usize::try_from(MAX_ENTRY_BYTES).unwrap() + 1024, b'a');
    let liar = tmp.path().join("liar.hikasa-pkg");
    package(
        &liar,
        &[("sessions/ok.toml", &big)],
        &[("sessions", "ok", "sessions/ok.toml")],
    );
    let mut bytes = std::fs::read(&liar).unwrap();
    for (signature, offset) in [(b"PK\x03\x04", 22), (b"PK\x01\x02", 24)] {
        let mut i = 0;
        while let Some(at) = bytes[i..].windows(4).position(|w| w == signature) {
            let at = i + at;
            bytes[at + offset..at + offset + 4].copy_from_slice(&100u32.to_le_bytes());
            i = at + 4;
        }
    }
    std::fs::write(&liar, &bytes).unwrap();
    let err = svc
        .settings_import(&liar, &ImportChoices::default(), false)
        .unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(!svc.config().dir.exists());
    // Five entries under the per-entry cap whose total is over 64 MiB.
    let mut part = b"type = \"text-compare\"\n#".to_vec();
    part.resize(15 << 20, b'a');
    let names: Vec<String> = (0..5).map(|i| format!("sessions/s{i}.toml")).collect();
    let stems: Vec<String> = (0..5).map(|i| format!("s{i}")).collect();
    let entries: Vec<(&str, &[u8])> = names
        .iter()
        .map(|n| (n.as_str(), part.as_slice()))
        .collect();
    let listed: Vec<(&str, &str, &str)> = names
        .iter()
        .zip(&stems)
        .map(|(n, s)| ("sessions", s.as_str(), n.as_str()))
        .collect();
    let total = tmp.path().join("total.hikasa-pkg");
    package(&total, &entries, &listed);
    let err = svc
        .settings_import(&total, &ImportChoices::default(), false)
        .unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_data", "{err}");
    assert!(err.to_string().contains("uncompressed"), "{err}");
    assert!(!svc.config().dir.exists());
}

/// Review W2 #3: `--item` alone selects its categories: only that item is exported; `system_info:system-info`
/// exports the system information.
#[test]
fn export_items_select_their_categories() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = populate(tmp.path());
    let out = tmp.path().join("one.hikasa-pkg");
    let dto = svc
        .settings_export(
            &ExportRequest {
                items: BTreeMap::from([(Category::Sessions, vec!["work/api".to_owned()])]),
                ..ExportRequest::default()
            },
            &out,
        )
        .unwrap();
    let paths: Vec<&str> = dto.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["sessions/work/api.toml"]);
    let dto = svc
        .settings_export(
            &ExportRequest {
                items: BTreeMap::from([(Category::SystemInfo, vec!["system-info".to_owned()])]),
                ..ExportRequest::default()
            },
            &out,
        )
        .unwrap();
    let paths: Vec<&str> = dto.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["system-info.txt"]);
}

/// Review W2 #10 (D-84): auto-save never overwrites a locked slot file.
#[test]
fn autosave_never_writes_a_locked_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 10);
    let s = Session::builtin(SessionType::TextCompare);
    let id = store.autosave(None, &s, &at(0, 0)).unwrap().unwrap();
    let path = store.autosaved_path(&id);
    let locked = std::fs::read_to_string(&path)
        .unwrap()
        .replace("type =", "locked = true\ntype =");
    std::fs::write(&path, &locked).unwrap();
    let mut changed = s.clone();
    changed.description = "changed".into();
    let again = store.autosave(Some(&id), &changed, &at(1, 0)).unwrap();
    assert_eq!(again, Some(id));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), locked);
}

/// D-90: a key the file holds stays written even when it equals the defaults later; a key it does not hold is
/// written only when it differs; `--unset` removes a key (the session follows the defaults again); an optional
/// rule that is off where the defaults set it is written `false` and reads back as off; nested unknown keys
/// under `[rules]` are kept (D-70).
#[test]
fn explicit_rule_keys_stay_written() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let store = svc.session_store();
    let kind = SessionType::FolderCompare;
    // The session sets flatten = true; then the defaults set it too: the key stays in the session's file.
    let mut session = Session::new(kind, &store.defaults(kind).unwrap());
    let SessionRules::Folder(rules) = &mut session.rules else {
        panic!("folder rules")
    };
    rules.handling.flatten = true;
    let path = store
        .save(&r("own"), &session, Overwrite::IfUnchanged)
        .unwrap();
    let mut defaults = Session::builtin(kind);
    let SessionRules::Folder(d) = &mut defaults.rules else {
        panic!("folder rules")
    };
    d.handling.flatten = true;
    d.filters.other.size = Some(hikasa_folder::SizeFilter {
        cmp: hikasa_folder::Cmp::Less,
        bytes: 10,
    });
    store.save_defaults(&defaults).unwrap();
    let loaded = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    store.save(&r("own"), &loaded, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("flatten = true"), "{text}");
    // A key not in the file and equal to the defaults is not written.
    assert!(!text.contains("size"), "{text}");
    // Off where the defaults set a size filter: `size = false`, read back as off.
    let mut off = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    let SessionRules::Folder(rules) = &mut off.rules else {
        panic!("folder rules")
    };
    assert!(rules.filters.other.size.is_some());
    rules.filters.other.size = None;
    store.save(&r("own"), &off, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("size = false"), "{text}");
    let back = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    let SessionRules::Folder(rules) = &back.rules else {
        panic!("folder rules")
    };
    assert_eq!(rules.filters.other.size, None);
    // `--unset` removes the keys: the session follows the defaults again.
    let edit = hikasa_session::api::SessionEdit {
        unset: vec![
            "rules.handling.flatten".into(),
            "rules.filters.other.size".into(),
        ],
        ..Default::default()
    };
    svc.session_edit(
        &hikasa_session::api::SessionTarget::Named(r("own")),
        &edit,
        false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("flatten"), "{text}");
    assert!(!text.contains("size"), "{text}");
    let back = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    let SessionRules::Folder(rules) = &back.rules else {
        panic!("folder rules")
    };
    assert!(rules.handling.flatten);
    assert_eq!(rules.filters.other.size.map(|s| s.bytes), Some(10));
    // An unknown key nested under [rules] survives a save (D-70).
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "type = \"folder-compare\"",
        "type = \"folder-compare\"\n[rules.criteria]\nfuture_key = 3",
    );
    std::fs::write(&path, &text).unwrap();
    let loaded = store.load(&r("own"), LoadMode::Strict).unwrap().value;
    store.save(&r("own"), &loaded, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("future_key = 3"), "{text}");
    // An explicit `--set` of the default value stays written too.
    let edit = hikasa_session::api::SessionEdit {
        set: vec![("rules.handling.flatten".into(), "true".into())],
        ..Default::default()
    };
    svc.session_edit(
        &hikasa_session::api::SessionTarget::Named(r("own")),
        &edit,
        false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("flatten = true"), "{text}");
}

/// Review W2 #6: nested unknown keys under `[rules]` of a text session are kept by a save.
#[test]
fn unknown_rule_keys_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path(), 10);
    let path = store.dir().join("t.toml");
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(
        &path,
        "schema_version = 1\ntype = \"text-compare\"\n[rules.importance]\nfuture_key = 3\n[rules.future]\nx = [1, 2]\n",
    )
    .unwrap();
    let mut loaded = store.load(&r("t"), LoadMode::Strict).unwrap().value;
    loaded.description = "changed".into();
    store.save(&r("t"), &loaded, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let table: Table = toml::from_str(&text).unwrap();
    assert_eq!(
        table["rules"]["importance"]["future_key"],
        Value::Integer(3)
    );
    assert_eq!(
        table["rules"]["future"]["x"],
        Value::Array(vec![Value::Integer(1), Value::Integer(2)])
    );
}

/// Review W2 #4: specs set with `session edit --set specs.*` are stored like `--left`/`--right`: canonical
/// absolute, `-` and `clipboard://` not stored (`spec_not_saved`).
#[test]
fn edit_set_specs_are_stored_canonical() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    svc.session_new(&r("s"), SessionType::TextCompare, None, None, None)
        .unwrap();
    let target = hikasa_session::api::SessionTarget::Named(r("s"));
    let edit = |key: &str, value: &str| hikasa_session::api::SessionEdit {
        set: vec![(key.to_owned(), value.to_owned())],
        ..Default::default()
    };
    let dto = svc
        .session_edit(&target, &edit("specs.left", "rel/a.txt"), false)
        .unwrap();
    assert!(dto.warnings.is_empty());
    let left = svc.session_effective(&r("s")).unwrap().specs.left;
    assert!(
        Path::new(&left).is_absolute() && left.ends_with("a.txt") && left.contains("work"),
        "{left}"
    );
    for value in ["-", "clipboard://"] {
        let dto = svc
            .session_edit(&target, &edit("specs.right", value), false)
            .unwrap();
        assert_eq!(dto.warnings[0].code, WarningCode::SpecNotSaved, "{value}");
        assert_eq!(svc.session_effective(&r("s")).unwrap().specs.right, "");
    }
}

/// Review W2 #9: deleting or renaming a locked session needs `force`.
#[test]
fn locked_sessions_need_force_to_delete_or_rename() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    svc.session_new(&r("l"), SessionType::TextCompare, None, None, None)
        .unwrap();
    svc.session_lock(&r("L"), true).unwrap();
    let err = svc.session_delete(&r("l"), false).unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_input", "{err}");
    let err = svc.session_rename(&r("l"), &r("m"), false).unwrap_err();
    assert_eq!(err.kind().as_str(), "invalid_input", "{err}");
    svc.session_rename(&r("l"), &r("m"), true).unwrap();
    svc.session_delete(&r("M"), true).unwrap();
    assert!(svc.session_store().find("m").is_none());
}
