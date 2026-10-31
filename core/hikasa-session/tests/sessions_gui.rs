//! The Service calls the GUI adds in M2 W4b (§1.4.15, D-105): typed forms of what `session new`, `session edit` and
//! the W2 auto-save do (same files, compared byte for byte), the current session of a view, scopes, workspaces.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::service;
use hikasa_core::WarningCode;
use hikasa_session::Service;
use hikasa_session::api::{ApiError, SessionEdit, SessionTarget, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewKind};
use hikasa_session::sessions::{
    AutoId, ParentLink, Scope, Session, SessionName, SessionRef, SessionRules, SessionType,
};
use hikasa_session::workspaces::{RestoredTab, Workspace, WorkspaceTab, WorkspaceWindow};
use toml::{Table, Value};

fn r(text: &str) -> SessionRef {
    SessionRef::parse(text).unwrap()
}

fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let key = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(key, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// A Unix absolute path as this host spells an absolute path.
fn abs(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn sessions_dir(svc: &Service) -> std::path::PathBuf {
    svc.session_store().dir().to_path_buf()
}

/// A session with text rules that differ from the defaults.
fn changed_rules() -> SessionRules {
    let mut rules = hikasa_text::TextRules::default();
    rules.alignment.skew_tolerance = 77;
    SessionRules::Text(rules)
}

/// T11, D-105: `session_save` writes what `session new` and `session edit` write.
#[test]
fn session_save_writes_the_bytes_of_new_and_edit() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (cli, gui) = (service(a.path()), service(b.path()));
    let reference = r("work/api");
    // `new`, with the same absolute specs in both worlds.
    let left = &abs("/w/a.txt");
    let right = &abs("/w/b.txt");
    cli.session_new(
        &reference,
        SessionType::TextCompare,
        Some(left),
        Some(right),
        Some("the api"),
    )
    .unwrap();
    let mut session = Session::new(
        SessionType::TextCompare,
        &gui.session_defaults(SessionType::TextCompare).unwrap(),
    );
    session.specs.left = left.into();
    session.specs.right = right.into();
    session.description = "the api".into();
    let dto = gui.session_save(&reference, &session, false).unwrap();
    assert_eq!(dto.action, "new");
    assert_eq!(files(&sessions_dir(&cli)), files(&sessions_dir(&gui)));
    // `edit` of a rule and the description.
    cli.session_edit(
        &SessionTarget::Named(reference.clone()),
        &SessionEdit {
            set: vec![("rules.alignment.skew_tolerance".into(), "77".into())],
            description: Some("changed".into()),
            ..SessionEdit::default()
        },
        false,
    )
    .unwrap();
    let mut session = gui.session_load(&reference).unwrap().value;
    session.rules = changed_rules();
    session.description = "changed".into();
    let dto = gui.session_save(&reference, &session, false).unwrap();
    assert_eq!(dto.action, "edit");
    assert_eq!(files(&sessions_dir(&cli)), files(&sessions_dir(&gui)));
}

/// T11, D-105: `session_save_defaults` writes what `session edit --defaults` writes.
#[test]
fn session_save_defaults_writes_the_bytes_of_edit_defaults() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (cli, gui) = (service(a.path()), service(b.path()));
    cli.session_edit(
        &SessionTarget::Defaults(SessionType::TextCompare),
        &SessionEdit {
            set: vec![("rules.alignment.skew_tolerance".into(), "77".into())],
            ..SessionEdit::default()
        },
        false,
    )
    .unwrap();
    let mut defaults = gui.session_defaults(SessionType::TextCompare).unwrap();
    defaults.rules = changed_rules();
    gui.session_save_defaults(&defaults).unwrap();
    assert_eq!(files(&sessions_dir(&cli)), files(&sessions_dir(&gui)));
}

/// T11: `session_autosave` is the store's auto-save with the injected clock (same file, pruned).
#[test]
fn session_autosave_writes_the_store_file_and_keeps_the_slot() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (direct, gui) = (service(a.path()), service(b.path()));
    let session = Session::builtin(SessionType::FolderCompare);
    let now = direct.clock().now();
    let id = direct
        .session_store()
        .autosave(None, &session, &now)
        .unwrap()
        .unwrap();
    let got = gui.session_autosave(None, &session).unwrap().unwrap();
    assert_eq!(got, id);
    assert_eq!(files(&sessions_dir(&direct)), files(&sessions_dir(&gui)));
    // The slot is written again, no second file.
    let mut changed = session.clone();
    changed.description = "second".into();
    assert_eq!(
        gui.session_autosave(Some(&id), &changed).unwrap(),
        Some(id.clone())
    );
    assert_eq!(files(&sessions_dir(&gui)).len(), 1);
    assert_eq!(
        gui.session_load_autosaved(&id).unwrap().value.description,
        "second"
    );
    gui.session_delete_autosaved(&id).unwrap();
    assert!(files(&sessions_dir(&gui)).is_empty());
    gui.session_delete_autosaved(&id).expect("gone is no error");
}

#[test]
fn session_autosave_is_off_at_zero() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let session = Session::builtin(SessionType::TextCompare);
    let id = svc.session_autosave(None, &session).unwrap().unwrap();
    common::write(
        dir.path(),
        "config/options.toml",
        b"schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = 0\n",
    );
    assert_eq!(svc.session_autosave(None, &session).unwrap(), None);
    assert!(!svc.session_store().autosaved_path(&id).exists(), "pruned");
}

#[test]
fn load_and_defaults_report_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    assert!(matches!(
        svc.session_load(&r("nope")),
        Err(ApiError::NotFound(_))
    ));
    assert_eq!(
        svc.session_defaults(SessionType::TextCompare).unwrap(),
        Session::builtin(SessionType::TextCompare)
    );
    let bad = AutoId::parse("20260101-000000-1").unwrap();
    assert!(svc.session_load_autosaved(&bad).is_err());
}

fn text_request(svc: &Service, left: &str, right: &str) -> ViewRequest {
    svc.open_view_request(
        ViewKind::TextCompare,
        &[left.to_owned(), right.to_owned()],
        &OpenOptions::default(),
    )
    .unwrap()
}

/// D-120: the current session of a view, then the view of that session (the same request again).
#[test]
fn session_for_view_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut named = Session::builtin(SessionType::TextCompare);
    named.specs.left = abs("/w/a.txt");
    named.specs.right = abs("/w/b.txt");
    named.specs.format = "text".into();
    named.specs.left_encoding = "utf-8".into();
    named.rules = changed_rules();
    let view = svc.session_view_of(&named, "x".into()).unwrap();
    assert_eq!(view.kind, ViewKind::TextCompare);
    let mut layout = Table::new();
    layout.insert("thumbnail".into(), Value::Boolean(false));
    let mut display = Table::new();
    display.insert("filter".into(), Value::String("x".into()));
    let parent = ParentLink {
        session: "work".into(),
        left: "a".into(),
        right: "b".into(),
    };
    let (current, warnings) = svc
        .session_for_view(
            &view.request,
            layout.clone(),
            display.clone(),
            Some(parent.clone()),
        )
        .unwrap();
    assert!(warnings.is_empty());
    assert_eq!(current.specs, named.specs);
    assert_eq!(current.rules, named.rules);
    assert_eq!(current.view, layout);
    assert_eq!(current.display, display);
    assert_eq!(current.parent, Some(parent));
    assert!(!hikasa_session::sessions::is_dirty(&named, &current));

    // The defaults of the type stand in for a view without session rules.
    let mut defaults = svc.session_defaults(SessionType::TextCompare).unwrap();
    defaults.rules = changed_rules();
    svc.session_save_defaults(&defaults).unwrap();
    let plain = text_request(&svc, &abs("/w/a.txt"), &abs("/w/b.txt"));
    let (current, _) = svc
        .session_for_view(&plain, Table::new(), Table::new(), None)
        .unwrap();
    assert_eq!(current.rules, changed_rules());

    // `clipboard://` is not stored.
    let plain = text_request(&svc, "clipboard://", &abs("/w/b.txt"));
    let (current, warnings) = svc
        .session_for_view(&plain, Table::new(), Table::new(), None)
        .unwrap();
    assert_eq!(current.specs.left, "");
    assert_eq!(warnings[0].code, WarningCode::SpecNotSaved);
}

#[test]
fn session_for_view_of_a_folder_view_keeps_rules_and_text_rules() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut named = Session::builtin(SessionType::FolderCompare);
    named.specs.left = abs("/w/l");
    named.specs.right = abs("/w/r");
    if let SessionRules::Folder(rules) = &mut named.rules {
        rules.criteria.timestamps.tolerance_secs += 5;
    }
    let view = svc.session_view_of(&named, "x".into()).unwrap();
    let (current, _) = svc
        .session_for_view(&view.request, Table::new(), Table::new(), None)
        .unwrap();
    assert!(!hikasa_session::sessions::is_dirty(&named, &current));
}

fn parent_session(svc: &Service) -> Session {
    let mut folder = Session::builtin(SessionType::FolderCompare);
    folder.specs.left = abs("/w/l");
    folder.specs.right = abs("/w/r");
    let _ = svc;
    folder
}

fn child(session: &str) -> Session {
    let mut child = Session::builtin(SessionType::TextCompare);
    child.parent = Some(ParentLink {
        session: session.into(),
        left: "a.c".into(),
        right: "a.c".into(),
    });
    child
}

/// D-82 through the Service: each scope writes the file `apply_scope` names.
#[test]
fn apply_scope_writes_what_the_scope_says() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let store = svc.session_store();
    let parent = r("p");
    svc.session_save(&parent, &parent_session(&svc), false)
        .unwrap();
    let change = changed_rules();

    // This view only: nothing on disk.
    let before = files(store.dir());
    let mut view = child("p");
    let dto = svc
        .session_apply_scope(Scope::View, &change, &mut view, Some("p"))
        .unwrap();
    assert!(dto.writes.view && dto.written.is_empty());
    assert_eq!(view.rules, change);
    assert_eq!(files(store.dir()), before);

    // Defaults: `_defaults/text-compare.toml`, rules only.
    let dto = svc
        .session_apply_scope(Scope::Defaults, &change, &mut child("p"), Some("p"))
        .unwrap();
    assert_eq!(dto.written.len(), 1);
    assert_eq!(
        svc.session_defaults(SessionType::TextCompare)
            .unwrap()
            .rules,
        change
    );

    // Pair and type write the parent; the override of the same key is replaced.
    let dto = svc
        .session_apply_scope(Scope::Pair, &change, &mut child("p"), Some("p"))
        .unwrap();
    assert!(dto.writes.parent);
    svc.session_apply_scope(Scope::Pair, &change, &mut child("p"), Some("p"))
        .unwrap();
    svc.session_apply_scope(Scope::Type, &change, &mut child("p"), Some("p"))
        .unwrap();
    assert_eq!(svc.session_load(&parent).unwrap().value.overrides.len(), 2);

    // An auto-saved parent is written in its slot.
    let id = svc
        .session_autosave(None, &parent_session(&svc))
        .unwrap()
        .unwrap();
    let link = format!("_autosaved/{id}");
    svc.session_apply_scope(Scope::Type, &change, &mut child(&link), Some(&link))
        .unwrap();
    assert_eq!(
        svc.session_load_autosaved(&id)
            .unwrap()
            .value
            .overrides
            .len(),
        1
    );

    // A locked parent: in memory only.
    svc.session_lock(&parent, true).unwrap();
    let before = files(store.dir());
    let dto = svc
        .session_apply_scope(Scope::Pair, &change, &mut child("p"), Some("p"))
        .unwrap();
    assert!(dto.writes.parent_in_memory_only && !dto.writes.parent);
    assert_eq!(files(store.dir()), before);

    // No parent, or a parent that is not a folder session.
    assert!(matches!(
        svc.session_apply_scope(Scope::Pair, &change, &mut child("p"), None),
        Err(ApiError::InvalidArgument(_))
    ));
    svc.session_save(&r("t"), &Session::builtin(SessionType::TextCompare), false)
        .unwrap();
    assert!(matches!(
        svc.session_apply_scope(Scope::Type, &change, &mut child("t"), Some("t")),
        Err(ApiError::InvalidArgument(_))
    ));
}

#[test]
fn rename_folder_moves_its_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    svc.session_save(
        &r("a/s"),
        &Session::builtin(SessionType::TextCompare),
        false,
    )
    .unwrap();
    svc.session_rename_folder("a", "b", false).unwrap();
    assert!(svc.session_load(&r("b/s")).is_ok());
    assert!(svc.session_load(&r("a/s")).is_err());
    assert!(matches!(
        svc.session_rename_folder("nope", "c", false),
        Err(ApiError::NotFound(_))
    ));
}

/// D-91: a folder command stops at a lock in its subtree, at any depth; `force` goes through.
#[test]
fn folder_commands_refuse_locked_sessions_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let text = Session::builtin(SessionType::TextCompare);
    svc.session_save(&r("a/s"), &text, false).unwrap();
    svc.session_save(&r("a/deep/t"), &text, false).unwrap();
    svc.session_lock(&r("a/deep/t"), true).unwrap();
    let inside: Vec<String> = svc
        .session_folder_contents("a")
        .unwrap()
        .into_iter()
        .map(|e| format!("{}:{}", e.reference, e.locked))
        .collect();
    assert_eq!(inside, ["a/deep/t:true", "a/s:false"]);
    for result in [
        svc.session_rmdir("a", true, false),
        svc.session_rename_folder("a", "b", false),
    ] {
        match result {
            Err(ApiError::InvalidArgument(text)) => {
                assert!(text.contains("1 locked session(s): a/deep/t"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(svc.session_load(&r("a/deep/t")).is_ok(), "nothing happened");
    svc.session_rename_folder("a", "b", true).unwrap();
    assert!(svc.session_load(&r("b/deep/t")).unwrap().value.locked);
    svc.session_rmdir("b", true, true).unwrap();
    assert!(svc.session_load(&r("b/deep/t")).is_err());
}

#[test]
fn inline_tables_keep_what_the_previous_table_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let session = Session::builtin(SessionType::TextCompare);
    let previous: Table = toml::from_str(
        "type = \"text-compare\"\nfuture = 1\n[rules]\nfuture_rule = 2\n[rules.alignment]\nskew_tolerance = 0\n",
    )
    .unwrap();
    let text = toml::to_string(&svc.inline_table(&session, Some(&previous)).unwrap()).unwrap();
    assert!(
        text.contains("future = 1") && text.contains("future_rule = 2"),
        "{text}"
    );
    let bare = toml::to_string(&svc.inline_table(&session, None).unwrap()).unwrap();
    assert!(!bare.contains("future"));
}

#[test]
fn workspace_load_gives_the_tabs_as_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut named = Session::builtin(SessionType::TextCompare);
    named.specs.left = abs("/w/a");
    named.specs.right = abs("/w/b");
    svc.session_save(&r("n"), &named, false).unwrap();
    let inline = svc.inline_table(&named, None).unwrap();
    assert!(!inline.contains_key("schema_version"));
    let workspace = Workspace {
        shortcut: "Ctrl+1".into(),
        windows: vec![WorkspaceWindow {
            active_tab: 1,
            tabs: vec![
                WorkspaceTab {
                    session: "n".into(),
                    ..WorkspaceTab::default()
                },
                WorkspaceTab {
                    inline,
                    ..WorkspaceTab::default()
                },
                WorkspaceTab {
                    session: "gone".into(),
                    ..WorkspaceTab::default()
                },
            ],
            ..WorkspaceWindow::default()
        }],
    };
    let name = SessionName::parse("W").unwrap();
    svc.workspace_save(&name, &workspace).unwrap();
    let loaded = svc
        .workspace_load(&SessionName::parse("w").unwrap())
        .unwrap();
    assert_eq!(loaded.name, "W");
    assert_eq!(loaded.workspace, workspace);
    assert_eq!(loaded.windows[0].window.active_tab, 1);
    assert!(matches!(
        &loaded.windows[0].tabs[0],
        RestoredTab::Named { reference, .. } if reference.to_string() == "n"
    ));
    assert!(matches!(&loaded.windows[0].tabs[1], RestoredTab::Inline(s) if s.specs == named.specs));
    assert!(matches!(&loaded.windows[0].tabs[2], RestoredTab::Home));
    assert_eq!(loaded.warnings[0].code, WarningCode::SessionMissing);
    assert!(matches!(
        svc.workspace_load(&SessionName::parse("none").unwrap()),
        Err(ApiError::NotFound(_))
    ));
}

#[test]
fn a_view_of_a_session_needs_both_specs() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let session = Session::builtin(SessionType::TextCompare);
    assert!(matches!(
        svc.session_view_of(&session, "t".into()),
        Err(ApiError::InvalidArgument(_))
    ));
}
