//! M2 W6c acceptance through the Service: the sync calls of §1.6.16 (A6.31's Service half), `SessionRules::Sync` (A6.30:
//! defaults, effective rules, sparse writes, a package round trip, Opaque for the other types), the confirmation of a sync
//! (`sync_confirmations`), and `folder sync --gui` up to the IPC boundary (D-190).

mod common;

use std::sync::Arc;

use common::{facts, fixed_clock, job, os, write};
use hikasa_core::{ErrorKind, JobKind, WarningCode};
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{ConfirmMode, OpVerb, QuestionKind, ReadOnlySides};
use hikasa_folder::sync::{
    LeaveReason, OrphanReading, SyncAction, SyncMatrix, SyncOptions, SyncPreset, SyncRules,
};
use hikasa_session::api::{
    FolderCompareRequest, FolderSyncRequest, GuiOpenRequest, OpOverrides, PreparedFolderCompare,
    ViewRequest, ViewWords, sync_options,
};
use hikasa_session::ipc::{OpenOptions, SyncWords, ViewKind};
use hikasa_session::launcher::RecordingLauncher;
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType, effective_rules};
use hikasa_session::settings::{LoadMode, Overwrite};
use hikasa_session::{ApiError, Service, ServiceInit};

fn service(root: &std::path::Path) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(2),
        launcher: Arc::new(RecordingLauncher::default()),
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

fn compare_request(root: &std::path::Path) -> FolderCompareRequest {
    FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        ..FolderCompareRequest::default()
    }
}

fn prepared(
    service: &Service,
    root: &std::path::Path,
    sync: &SyncOptions,
) -> PreparedFolderCompare {
    std::fs::create_dir_all(root.join("l")).unwrap();
    std::fs::create_dir_all(root.join("r")).unwrap();
    let control = job(service, JobKind::FolderCompare);
    let prepared = service
        .folder_sync_prepare(&compare_request(root), sync, &control)
        .unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    prepared
}

fn mirror_right() -> SyncOptions {
    SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    }
}

fn tree(root: &std::path::Path) {
    write(root, "l/a.txt", b"one");
    write(root, "l/b.txt", b"left");
    write(root, "r/b.txt", b"right side");
    write(root, "r/z.txt", b"orphan");
}

#[test]
fn a6_31_service_prepare_plan_dry_run_and_execute() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let options = mirror_right();
    let prepared = prepared(&service, root, &options);
    // The sync's rules: every folder scanned, archives are files without `within_archives`.
    assert!(prepared.rules().handling.auto_scan_subfolders);
    assert_eq!(
        prepared.rules().handling.archives,
        hikasa_folder::ArchiveHandling::AsFiles
    );
    let request = FolderSyncRequest {
        options: options.clone(),
        ..FolderSyncRequest::default()
    };
    let control = job(&service, JobKind::FolderSync);
    let plan = service
        .folder_sync_plan(prepared.model(), &request, &control)
        .unwrap();
    assert_eq!(plan.op.verb, OpVerb::Sync);
    let dry = service.folder_sync_dry_run(plan.clone());
    assert!(dry.op.dry_run && dry.ok());
    assert!(root.join("r/z.txt").exists(), "a dry run wrote");
    let done = service.folder_sync_execute(plan, &control).unwrap();
    assert!(done.ok(), "{:?}", done.op.report.results);
    assert_eq!(done.counts.copy_right, 2);
    assert_eq!(done.counts.delete_right, 1);
    assert!(!root.join("r/z.txt").exists());
    assert_eq!(std::fs::read(root.join("r/b.txt")).unwrap(), b"left");
    let dto = done.dto(&prepared.model().read(), false);
    assert_eq!(dto.preset, "mirror_right");
    assert_eq!(dto.rows.len(), 3);
    assert_eq!(dto.items.len(), dry.op.report.plan.items.len());
    // A just-selection with nothing in it acts on nothing.
    let none = FolderSyncRequest {
        options,
        select: Some(hikasa_session::api::SelectRequest::default()),
        ..FolderSyncRequest::default()
    };
    let err = service
        .folder_sync_plan(prepared.model(), &none, &control)
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn a6_31_service_refusals_and_read_only_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "r/a.txt", b"right");
    let service = service(root);
    let options = mirror_right();
    let prepared = prepared(&service, root, &options);
    let control = job(&service, JobKind::FolderSync);
    let request = FolderSyncRequest {
        options: options.clone(),
        ..FolderSyncRequest::default()
    };
    let err = service
        .folder_sync_plan(prepared.model(), &request, &control)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err:?}");
    assert_eq!(
        err.exit_code(hikasa_session::api::ApiOp::FolderSync).code(),
        100
    );
    // Read-only right: the row is left, a warning says why.
    write(root, "l/b.txt", b"left");
    let prepared = prepared_again(&service, root, &options);
    let ro = FolderSyncRequest {
        options,
        read_only: ReadOnlySides {
            left: false,
            right: true,
        },
        ..FolderSyncRequest::default()
    };
    let plan = service
        .folder_sync_plan(prepared.model(), &ro, &control)
        .unwrap();
    assert!(plan.rows.iter().all(|r| r.action == SyncAction::Leave));
    assert!(
        plan.rows
            .iter()
            .any(|r| r.reason == Some(LeaveReason::ReadOnlySide))
    );
    let outcome = service.folder_sync_dry_run(plan);
    assert!(
        outcome
            .op
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::ReadOnlySide)
    );
    // A flat comparison is refused before anything is scanned.
    let mut flat = compare_request(root);
    flat.patch.flatten = Some(true);
    let err = service
        .folder_sync_prepare(
            &flat,
            &mirror_right(),
            &job(&service, JobKind::FolderCompare),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

fn prepared_again(
    service: &Service,
    root: &std::path::Path,
    sync: &SyncOptions,
) -> PreparedFolderCompare {
    prepared(service, root, sync)
}

#[test]
fn a6_31_a_sync_asks_by_sync_confirmations_without_a_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let options = mirror_right();
    let prepared = prepared(&service, root, &options);
    let control = job(&service, JobKind::FolderSync);
    let request = FolderSyncRequest {
        options,
        ..FolderSyncRequest::default()
    };
    let plan = service
        .folder_sync_plan(prepared.model(), &request, &control)
        .unwrap();
    // `prompt` (the default): the sync question comes first.
    let machine = service.folder_confirmation(&plan.op, None);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::Operation);
    // `sync_confirmations = "yes_to_all"`: nothing asked; a mode given still wins.
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("config/options.toml"),
        "schema_version = 1\n[file_operations]\nsync_confirmations = \"yes_to_all\"\n",
    )
    .unwrap();
    assert!(service.folder_confirmation(&plan.op, None).next().is_none());
    let asked = service.folder_confirmation(&plan.op, Some(ConfirmMode::Prompt));
    assert!(asked.next().is_some());
    // A script's safety never depends on the file (D-77): the strict machine asks.
    assert!(
        service
            .folder_confirmation_strict(&plan.op, Some(ConfirmMode::Prompt))
            .next()
            .is_some()
    );
}

// ---- sessions --------------------------------------------------------------------------------------------------

#[test]
fn a6_30_sync_rules_of_a_session() {
    // A folder-sync session has typed rules; the other types of W12 stay opaque.
    let sync = Session::builtin(SessionType::FolderSync);
    let SessionRules::Sync(rules) = &sync.rules else {
        panic!("{:?}", sync.rules);
    };
    assert_eq!(rules.sync.preset, SyncPreset::UpdateRight);
    for kind in [
        SessionType::TableCompare,
        SessionType::HexCompare,
        SessionType::PictureCompare,
    ] {
        assert!(
            matches!(Session::builtin(kind).rules, SessionRules::Opaque(_)),
            "{kind}"
        );
    }
    // The effective rules of a session over the type's defaults: the defaults' preset, the session's own change on top.
    let mut defaults = Session::builtin(SessionType::FolderSync);
    if let SessionRules::Sync(rules) = &mut defaults.rules {
        rules.sync.preset = SyncPreset::MirrorLeft;
        rules.sync.recycle = true;
    }
    // A session made from the type's defaults, which then changes one option.
    let mut child = Session::new(SessionType::FolderSync, &defaults);
    if let SessionRules::Sync(rules) = &mut child.rules {
        rules.sync.create_empty = true;
    }
    let SessionRules::Sync(effective) = effective_rules(&defaults, None, &child) else {
        panic!("not sync rules");
    };
    assert_eq!(effective.sync.preset, SyncPreset::MirrorLeft);
    assert!(effective.sync.recycle && effective.sync.create_empty);
    // A round trip through the value of a session file.
    let mut rules = SyncRules::default();
    rules.sync.preset = SyncPreset::Custom;
    rules.sync.matrix = SyncMatrix::parse("newer-left=copy-right", SyncMatrix::default()).unwrap();
    rules.sync.orphans = OrphanReading::Deleted;
    rules.folder.filters.other.size = Some(hikasa_folder::SizeFilter {
        cmp: hikasa_folder::Cmp::Greater,
        bytes: 10,
    });
    let value = SessionRules::Sync(rules.clone()).to_value();
    let back = SessionRules::from_value(&SessionRules::Sync(SyncRules::default()), value).unwrap();
    assert_eq!(back, SessionRules::Sync(rules));
}

#[test]
fn a6_30_sparse_writes_and_read_only_sides_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let store = service.session_store();
    let reference = SessionRef::parse("s").unwrap();
    let mut session = Session::builtin(SessionType::FolderSync);
    session.specs.left = "/a".into();
    session.specs.right = "/b".into();
    session.specs.read_only_right = true;
    if let SessionRules::Sync(rules) = &mut session.rules {
        rules.sync.preset = SyncPreset::MirrorRight;
        rules.sync.overrides.insert("x/y".into(), SyncAction::Leave);
    }
    store.save(&reference, &session, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(store.path(&reference)).unwrap();
    assert!(text.contains("preset = \"mirror_right\""), "{text}");
    assert!(text.contains("read_only_right = true"), "{text}");
    assert!(!text.contains("create_empty"), "sparse: {text}");
    let loaded = store.load(&reference, LoadMode::Strict).unwrap().value;
    assert_eq!(loaded, session);
    // An override removed in memory is gone from the file (a map's entries are known keys).
    let mut edited = loaded;
    if let SessionRules::Sync(rules) = &mut edited.rules {
        rules.sync.overrides.clear();
    }
    store.save(&reference, &edited, Overwrite::Force).unwrap();
    let text = std::fs::read_to_string(store.path(&reference)).unwrap();
    assert!(!text.contains("x/y"), "{text}");
    // Swap Sides swaps the read-only sides.
    edited.swap_sides();
    assert!(edited.specs.read_only_left && !edited.specs.read_only_right);
    // A key the typed rules do not know stays in the file across a save (D-70).
    let path = store.path(&reference);
    let mut with_unknown = std::fs::read_to_string(&path).unwrap();
    with_unknown.push_str("\n[rules.sync.later]\nkey = 1\n");
    std::fs::write(&path, &with_unknown).unwrap();
    let loaded = store.load(&reference, LoadMode::Strict).unwrap().value;
    store.save(&reference, &loaded, Overwrite::Force).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("[rules.sync.later]")
    );
}

#[test]
fn a6_30_package_round_trip_of_a_folder_sync_session() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("one");
    let service_one = service(&root);
    let store = service_one.session_store();
    let reference = SessionRef::parse("nightly").unwrap();
    let mut session = Session::builtin(SessionType::FolderSync);
    session.specs.left = "/a".into();
    session.specs.right = "/b".into();
    session.specs.read_only_left = true;
    if let SessionRules::Sync(rules) = &mut session.rules {
        rules.sync.preset = SyncPreset::Custom;
        rules.sync.matrix.newer_left = SyncAction::CopyRight;
        rules.sync.within_archives = true;
        rules
            .sync
            .overrides
            .insert("keep".into(), SyncAction::Leave);
    }
    store.save(&reference, &session, Overwrite::Force).unwrap();
    let package = tmp.path().join("p.hikasa-pkg");
    service_one
        .settings_export(&hikasa_session::package::ExportRequest::default(), &package)
        .unwrap();
    let two = service(&tmp.path().join("two"));
    let result = two
        .settings_import(
            &package,
            &hikasa_session::package::ImportChoices::default(),
            false,
        )
        .unwrap();
    assert!(result.ok(), "{result:?}");
    let back = two
        .session_store()
        .load(&reference, LoadMode::Strict)
        .unwrap()
        .value;
    assert_eq!(back, session);
    assert_eq!(
        std::fs::read(two.session_store().path(&reference)).unwrap(),
        std::fs::read(store.path(&reference)).unwrap()
    );
}

// ---- the GUI boundary (D-190) ----------------------------------------------------------------------------------

#[test]
fn a6_31_sync_words_cross_the_ipc_as_words_and_parse_on_both_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let words = SyncWords {
        preset: Some("mirror-left".into()),
        orphans: Some("deleted".into()),
        create_empty: true,
        overrides: vec!["a.txt=leave".into()],
        allow_empty_source: true,
        ..SyncWords::default()
    };
    let open =
        GuiOpenRequest::for_sync(&compare_request(root), &words, &ViewWords::default(), false);
    assert_eq!(open.kind, Some(ViewKind::FolderSync));
    assert_eq!(open.options.sync.as_ref(), Some(&words));
    let resolved = service.resolve_open(&open).unwrap();
    assert_eq!(resolved.kind, ViewKind::FolderSync);
    service.check_open(&resolved).unwrap();
    let ViewRequest::Sync {
        options,
        allow_empty_source,
        ..
    } = service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .unwrap()
    else {
        panic!("not a sync request");
    };
    assert_eq!(options.preset, SyncPreset::MirrorLeft);
    assert_eq!(options.orphans, OrphanReading::Deleted);
    assert!(options.create_empty && allow_empty_source);
    assert_eq!(options.overrides.get("a.txt"), Some(&SyncAction::Leave));
    // The same words through JSON, as the IPC carries them; an `OpenOptions` without them has neither key.
    let text = serde_json::to_string(&resolved.options).unwrap();
    let again: OpenOptions = serde_json::from_str(&text).unwrap();
    assert_eq!(again, resolved.options);
    let plain = serde_json::to_string(&OpenOptions::default()).unwrap();
    assert!(
        !plain.contains("sync") && !plain.contains("no_backups"),
        "{plain}"
    );
    // A bad word is refused before any contact (check_open).
    let bad = GuiOpenRequest::for_sync(
        &compare_request(root),
        &SyncWords {
            custom: Some("added-left=copy-left".into()),
            ..SyncWords::default()
        },
        &ViewWords::default(),
        false,
    );
    let resolved = service.resolve_open(&bad).unwrap();
    assert!(service.check_open(&resolved).is_err());
    // The GUI's answer to a sync view.
    assert_eq!(
        hikasa_session::ipc::unserved_view(ViewKind::FolderSync),
        "this window cannot show a Folder Sync view yet"
    );
    // One parser for the CLI and the IPC: the session's matrix is the base of `--custom`.
    let base = SyncOptions {
        matrix: SyncMatrix::of(SyncPreset::UpdateLeft).unwrap(),
        ..SyncOptions::default()
    };
    let custom = sync_options(
        &SyncWords {
            custom: Some("same=leave".into()),
            ..SyncWords::default()
        },
        &base,
        false,
    )
    .unwrap();
    assert_eq!(custom.preset, SyncPreset::Custom);
    assert_eq!(custom.matrix, base.matrix);
    // `--override` on Windows takes `\` too.
    let windows = sync_options(
        &SyncWords {
            overrides: vec![r"a\b.txt=leave".into()],
            ..SyncWords::default()
        },
        &SyncOptions::default(),
        true,
    )
    .unwrap();
    assert!(windows.overrides.contains_key("a/b.txt"));
    let _ = OpOverrides::default();
}
