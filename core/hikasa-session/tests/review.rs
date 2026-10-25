//! Regression tests of the wave-4 session review (§1.8.7): pool panics, the effective folder
//! ignore-unimportant in JSON and reports, the private stdin spool, the GUI launch arguments and the GUI
//! executable override, the per-field unix criteria, the `--gui` ↔ `OpenView` mappings, lazy standard input,
//! the D-23 close rules, FC-062 non-UTF-8 pairs, second-instance forwarding, local validation of `--gui`, and
//! the minor fixes (rules-file errors, options.toml edge cases, switch parsing, spool and atomic-write names,
//! format tokens ranges, report paths that depend on the grammar).

mod common;

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{CloseMode, FakeGui, FakeLauncher, job, os, service, service_with, write};
use hikasa_core::{
    CompareVerdict, ContentKind, ContentMethod, ContentResult, ExitCode, JobKind, Spec, WarningCode,
};
use hikasa_folder::{
    DisplayOptions, FolderDisplayFilter, FolderMode, FolderRulesPatch, NullObserver,
};
use hikasa_report::{FolderLayout, FolderReportOptions, PatchStyle, TextLayout, TextReportOptions};
use hikasa_session::api::dto::{JsonEnvelope, QUICK_SCHEMA, QuickDto, USAGE_ERROR_SCHEMA};
use hikasa_session::api::{
    FolderCompareRequest, FormatTokensRequest, GuiOpenOutcome, GuiOpenRequest, QuickMethod,
    QuickRequest, ReportSettings, StdinSource, TextCompareRequest, ViewOutcome, ViewRequest,
    ViewWords, find_gui_exe, open_usage, parse_open_args,
};
use hikasa_session::ipc::{CloseReason, IpcServer, Request, RetryPolicy, ViewKind};
use hikasa_session::{ApiError, ApiOp, EnvFacts, Platform, Service, ServiceInit};
use hikasa_text::{TextDisplayFilter, TextRulesPatch, UnimportantItem, ViewOptions};

fn start(service: &Service, gui: Arc<FakeGui>) -> hikasa_session::ipc::ServerHandle {
    IpcServer::bind(&service.endpoint().unwrap())
        .unwrap()
        .spawn(gui)
        .unwrap()
}

#[test]
fn a_panic_in_pool_spawn_is_logged_and_the_pool_keeps_working() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    // Without a panic handler rayon aborts the whole process here (decision 3).
    service
        .pool()
        .spawn(|| panic!("a GUI background task failed"));
    let (tx, rx) = crossbeam_channel::bounded(1);
    service.pool().spawn(move || {
        let _ = tx.send(42);
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), 42);
    // Work that must report its panic goes through spawn: it becomes ApiError::Internal.
    let handle = service.spawn(JobKind::Report, "boom".into(), |service, _| {
        service
            .pool()
            .install(|| -> Result<(), ApiError> { panic!("inside install") })
    });
    assert!(matches!(handle.wait(), Err(ApiError::Internal(m)) if m.contains("inside install")));
}

#[test]
fn the_criteria_word_ignore_unimportant_reaches_json_and_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/a.c", b"int a;\n");
    write(&w, "r/a.c", b"int  a;\n");
    let resp = service
        .folder_compare(
            &FolderCompareRequest {
                left: os(&w.join("l")),
                right: os(&w.join("r")),
                criteria: Some("rules-based ignore-unimportant".into()),
                text_patch: TextRulesPatch {
                    unimportant: vec![UnimportantItem::Whitespace],
                    ..TextRulesPatch::default()
                },
                ..FolderCompareRequest::default()
            },
            &job(&service, JobKind::FolderCompare),
            &NullObserver,
        )
        .unwrap();
    assert!(resp.ignore_unimportant);
    assert_eq!(resp.verdict, CompareVerdict::NoDifferences);
    assert_eq!(resp.exit_code, ExitCode::RulesSame);
    // The adapter's display says "not ignoring" (it does not parse --criteria): the JSON still agrees.
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::AlwaysShowFolders,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let json = serde_json::to_value(resp.json(&display)).unwrap();
    assert_eq!(json["verdict"], resp.verdict.as_str());
    assert_eq!(json["display"]["ignore_unimportant"], true);
    let opts = FolderReportOptions {
        layout: FolderLayout::List,
        display,
        ..FolderReportOptions::default()
    };
    let mut out = Vec::new();
    service.render_folder(&resp, &opts, &mut out).unwrap();
    let list = String::from_utf8(out).unwrap();
    assert!(list.contains("=  a.c"), "{list}");
    assert!(
        !list.contains("~"),
        "no unimportant marker under iu: {list}"
    );
}

#[test]
fn quick_and_text_responses_carry_their_ignore_unimportant() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.c", b"int a;\n");
    let b = write(&w, "b.c", b"int  a;\n");
    let patch = TextRulesPatch {
        unimportant: vec![UnimportantItem::Whitespace],
        ..TextRulesPatch::default()
    };
    let quick = service
        .quick(
            &QuickRequest {
                left: os(&a),
                right: os(&b),
                method: QuickMethod::Rules,
                ignore_unimportant: true,
                rules_patch: patch.clone(),
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap();
    assert!(quick.ignore_unimportant);
    let dto = serde_json::to_value(QuickDto::new(&quick)).unwrap();
    assert_eq!(dto["ignore_unimportant"], true);
    let envelope =
        JsonEnvelope::success(QUICK_SCHEMA, quick.exit_code, QuickDto::new(&quick), vec![]);
    assert_eq!(envelope.exit_code, 2);
    let text = service
        .text_compare(
            &TextCompareRequest {
                left: os(&a),
                right: os(&b),
                rules_patch: patch,
                ignore_unimportant: true,
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert!(text.ignore_unimportant);
    let json = serde_json::to_value(text.json(&ViewOptions::default())).unwrap();
    assert_eq!(json["ignore_unimportant"], true);
    assert_eq!(json["verdict"]["result"], "rules_same");
    assert_eq!(text.verdict, ContentResult::RulesSame);
    assert_eq!(text.exit_code, ExitCode::RulesSame);
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// A service whose cache folder is the per-user temp fallback (no home, no XDG cache), with stdin.
fn fallback_cache_service(root: &Path, stdin: &[u8], name: &str) -> Service {
    let facts = EnvFacts {
        home: None,
        xdg_config_home: Some(root.join("xdgcfg")),
        xdg_cache_home: None,
        ..common::facts(root)
    };
    let init = ServiceInit {
        stdin: Some(StdinSource::from_bytes(
            stdin.to_vec(),
            Some(name.to_owned()),
        )),
        ..ServiceInit::new(
            facts,
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

#[cfg(unix)]
#[test]
fn the_stdin_spool_is_private_and_refuses_planted_links() {
    let tmp = common::short_tempdir();
    let root = tmp.path();
    let service = fallback_cache_service(root, b"password=hunter2\n", "secret.txt");
    let cache = service.config().cache_dir.clone();
    assert!(cache.starts_with(root.join("tmp")), "{cache:?}");
    assert!(
        cache
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hikasa-cache-")
    );
    let other = write(&root.join("work"), "other.txt", b"x\n");
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = start(&service, Arc::clone(&gui));
    let report = service
        .gui_open_with(
            &GuiOpenRequest {
                specs: vec![OsString::from("-"), os(&other)],
                ..GuiOpenRequest::default()
            },
            None,
            &RetryPolicy::default(),
        )
        .unwrap();
    let spooled = report.spooled_stdin.clone().unwrap();
    assert_eq!(std::fs::read(&spooled).unwrap(), b"password=hunter2\n");
    assert_eq!(mode_of(&spooled), 0o600);
    assert_eq!(mode_of(spooled.parent().unwrap()), 0o700);
    assert_eq!(mode_of(&cache.join("stdin")), 0o700);
    assert_eq!(mode_of(&cache) & 0o022, 0);
    server.shutdown().unwrap();

    // Another user planted a symbolic link as the shared fallback folder: refused, nothing written there.
    let tmp = common::short_tempdir();
    let root = tmp.path();
    let service = fallback_cache_service(root, b"secret\n", "s.txt");
    let attacker = root.join("attacker");
    std::fs::create_dir_all(&attacker).unwrap();
    std::os::unix::fs::symlink(&attacker, &service.config().cache_dir).unwrap();
    let other = write(&root.join("work"), "other.txt", b"x\n");
    let err = service
        .gui_open(
            &GuiOpenRequest {
                specs: vec![OsString::from("-"), os(&other)],
                ..GuiOpenRequest::default()
            },
            None,
        )
        .unwrap_err();
    assert!(err.to_string().contains("symbolic link"), "{err}");
    assert_eq!(std::fs::read_dir(&attacker).unwrap().count(), 0);

    // A planted `stdin` link inside a real cache folder is refused too.
    std::fs::remove_file(&service.config().cache_dir).unwrap();
    std::fs::create_dir(&service.config().cache_dir).unwrap();
    std::os::unix::fs::symlink(&attacker, service.config().cache_dir.join("stdin")).unwrap();
    assert!(
        service
            .gui_open(
                &GuiOpenRequest {
                    specs: vec![OsString::from("-"), os(&other)],
                    ..GuiOpenRequest::default()
                },
                None,
            )
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&attacker).unwrap().count(), 0);
}

#[test]
fn a_failed_spool_leaves_nothing_behind_and_long_valid_names_work() {
    let tmp = common::short_tempdir();
    let root = tmp.path();
    let other = write(&root.join("work"), "o.c", b"int a;\n");
    let long = format!("{}.c", "n".repeat(300));
    let service = service_with(
        root,
        Some(common::blob(b"int a;\n", Some(&long))),
        Arc::new(common::UpperRunner::default()),
    );
    let err = service
        .gui_open(
            &GuiOpenRequest {
                specs: vec![OsString::from("-"), os(&other)],
                wait: true,
                ..GuiOpenRequest::default()
            },
            None,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::Session(_)), "{err:?}");
    let stdin_root = service.config().cache_dir.join("stdin");
    assert_eq!(std::fs::read_dir(&stdin_root).unwrap().count(), 0);
    // A 250-byte name (valid on every common file system) is spooled.
    let name = format!("{}.txt", "n".repeat(246));
    let service = service_with(
        &root.join("second"),
        Some(common::blob(b"x\n", Some(&name))),
        Arc::new(common::UpperRunner::default()),
    );
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Similar);
    let server = start(&service, gui);
    let report = service
        .gui_open_with(
            &GuiOpenRequest {
                specs: vec![OsString::from("-"), os(&other)],
                wait: true,
                ..GuiOpenRequest::default()
            },
            None,
            &RetryPolicy::default(),
        )
        .unwrap();
    assert_eq!(report.exit_code(), 12);
    server.shutdown().unwrap();
    // write_atomic with the same name (its temporary name is bounded).
    let target = root.join(&name);
    hikasa_session::write_atomic(&target, b"atomic").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"atomic");
}

#[test]
fn the_launch_forwards_the_configuration_and_the_endpoint_override() {
    let tmp = common::short_tempdir();
    let explicit = service(tmp.path());
    let launch = explicit.gui_launch();
    assert_eq!(
        launch.args,
        vec![
            OsString::from("--config-dir"),
            explicit.config().dir.clone().into_os_string()
        ]
    );
    assert_eq!(
        launch.env,
        vec![(
            OsString::from("HIKASA_IPC_ENDPOINT"),
            explicit.env().ipc_endpoint_override.clone().unwrap()
        )]
    );
    // The per-user default needs no argument: the GUI resolves the same folder.
    let mut facts = common::facts(tmp.path());
    facts.ipc_endpoint_override = None;
    let user = Service::new(ServiceInit::new(
        facts,
        common::fixed_clock(),
        Arc::new(common::UpperRunner::default()),
    ))
    .unwrap();
    assert!(user.gui_launch().args.is_empty() && user.gui_launch().env.is_empty());
}

/// A launcher that behaves like `hikasa-gui`: it resolves its configuration from the launch arguments (the
/// shared parser), computes its endpoint and starts serving there.
struct LikeTheGui {
    facts: EnvFacts,
    launched: AtomicUsize,
    server: parking_lot::Mutex<Option<hikasa_session::ipc::ServerHandle>>,
}

impl hikasa_session::api::GuiLauncher for LikeTheGui {
    fn launch(
        &self,
        launch: &hikasa_session::api::GuiLaunch,
    ) -> Result<(), hikasa_session::api::LaunchError> {
        self.launched.fetch_add(1, Ordering::SeqCst);
        let args = parse_open_args(&launch.args, Platform::HOST).unwrap();
        let config =
            hikasa_session::resolve_config_dir(&self.facts, args.config_dir.as_deref(), &|_| false)
                .unwrap();
        let endpoint = hikasa_session::ipc::Endpoint::for_config(&self.facts, &config).unwrap();
        let server = IpcServer::bind(&endpoint)
            .unwrap()
            .spawn(FakeGui::new(CloseMode::Immediately, ExitCode::Similar))
            .unwrap();
        *self.server.lock() = Some(server);
        Ok(())
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d.min(Duration::from_millis(5)));
    }
}

#[cfg(unix)]
#[test]
fn a_launched_gui_serves_an_explicit_configuration() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = common::short_tempdir();
    let runtime = tmp.path().join("run");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let facts = EnvFacts {
        ipc_endpoint_override: None,
        xdg_runtime_dir: Some(runtime.clone()),
        xdg_config_home: Some(tmp.path().join("xdg")),
        ..common::facts(tmp.path())
    };
    let init = ServiceInit {
        config_dir: Some(tmp.path().join("cfg")),
        ..ServiceInit::new(
            facts.clone(),
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    assert!(!service.config().is_default);
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let b = write(&w, "b.txt", b"b\n");
    let launcher = LikeTheGui {
        facts,
        launched: AtomicUsize::new(0),
        server: parking_lot::Mutex::new(None),
    };
    let policy = RetryPolicy {
        total: Duration::from_secs(2),
        ..RetryPolicy::default()
    };
    let report = service
        .gui_open_with(
            &GuiOpenRequest {
                specs: vec![os(&a), os(&b)],
                wait: true,
                ..GuiOpenRequest::default()
            },
            Some(&launcher),
            &policy,
        )
        .unwrap();
    assert_eq!(launcher.launched.load(Ordering::SeqCst), 1);
    assert_eq!(
        report.exit_code(),
        12,
        "the launched GUI is reachable (no 102)"
    );
    if let Some(server) = launcher.server.lock().take() {
        server.shutdown().unwrap();
    }
}

#[test]
fn the_gui_executable_override_comes_first_and_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let sibling = write(tmp.path(), "bin/hikasa-gui", b"#!/bin/sh\n");
    let fake = write(tmp.path(), "fake/my-gui", b"#!/bin/sh\n");
    let facts = EnvFacts {
        exe_dir: Some(tmp.path().join("bin")),
        ..EnvFacts::new(Platform::Linux, tmp.path().to_path_buf())
    };
    assert_eq!(find_gui_exe(&facts), Some(sibling));
    let overridden = EnvFacts {
        gui_exe_override: Some(fake.clone()),
        ..facts.clone()
    };
    assert_eq!(find_gui_exe(&overridden), Some(fake));
    // An override that does not exist means "not found" (exit 103), never the sibling.
    let missing = EnvFacts {
        gui_exe_override: Some(tmp.path().join("missing")),
        ..facts
    };
    assert_eq!(find_gui_exe(&missing), None);
}

#[test]
fn unix_criteria_flags_are_independent_of_the_rules_file() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/u.txt", b"u\n");
    write(&w, "r/u.txt", b"u\n");
    let rules = write(&w, "unix.toml", b"[criteria.unix]\npermissions = true\n");
    let prepared = service
        .folder_prepare(&FolderCompareRequest {
            left: os(&w.join("l")),
            right: os(&w.join("r")),
            rules_file: Some(os(&rules)),
            patch: FolderRulesPatch {
                unix_owner: Some(true),
                ..FolderRulesPatch::default()
            },
            ..FolderCompareRequest::default()
        })
        .unwrap();
    let unix = prepared.rules().criteria.unix;
    assert!(unix.permissions && unix.owner && !unix.group, "{unix:?}");
    // `--criteria permissions --owner` keeps permissions too.
    let prepared = service
        .folder_prepare(&FolderCompareRequest {
            left: os(&w.join("l")),
            right: os(&w.join("r")),
            criteria: Some("permissions".into()),
            patch: FolderRulesPatch {
                unix_owner: Some(true),
                ..FolderRulesPatch::default()
            },
            ..FolderCompareRequest::default()
        })
        .unwrap();
    let unix = prepared.rules().criteria.unix;
    assert!(unix.permissions && unix.owner && !unix.group, "{unix:?}");
}

#[test]
fn gui_requests_and_open_views_map_both_ways() {
    let tmp = common::short_tempdir();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "a.txt", b"caf\xc3\xa9\n");
    write(&w, "b.txt", b"caf\xc3\x83\xc2\xa9\n");
    write(
        &w,
        "rules.toml",
        b"[importance]\nembedded_whitespace = false\n",
    );
    let text = TextCompareRequest {
        left: "a.txt".into(),
        right: "b.txt".into(),
        format: Some("c-cpp".into()),
        encodings: hikasa_core::Pair::new(Some("windows-1252".into()), None),
        rules_file: Some("rules.toml".into()),
        rules_patch: TextRulesPatch {
            skew_tolerance: Some(10),
            ..TextRulesPatch::default()
        },
        ignore_unimportant: true,
    };
    let words = ViewWords {
        display: Some("context".into()),
        context_lines: Some(2),
        ..ViewWords::default()
    };
    let open = GuiOpenRequest::for_text(&text, &words, true);
    assert_eq!(open.kind, Some(ViewKind::TextCompare));
    let resolved = service.resolve_open(&open).unwrap();
    assert!(
        resolved
            .specs
            .iter()
            .all(|s| s.starts_with('/') || s.contains(":\\"))
    );
    let rules_file = resolved.options.text_rules_file.clone().unwrap();
    assert!(Path::new(&rules_file).is_absolute(), "{rules_file}");
    service.check_open(&resolved).unwrap();
    // The GUI side: the same request comes back, with the view options of the words.
    match service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .unwrap()
    {
        ViewRequest::Text { request, view } => {
            assert_eq!(request.encodings.left.as_deref(), Some("windows-1252"));
            assert_eq!(request.encodings.right, None);
            assert_eq!(request.format.as_deref(), Some("c-cpp"));
            assert_eq!(request.rules_patch.skew_tolerance, Some(10));
            assert!(request.ignore_unimportant);
            assert_eq!(view.filter, TextDisplayFilter::Context { lines: 2 });
            assert!(view.ignore_unimportant);
            // The view compares with the override, like the local compare (D-50).
            let resp = service
                .text_compare(&request, &job(&service, JobKind::TextCompare), None)
                .unwrap();
            let local = service
                .text_compare(&text, &job(&service, JobKind::TextCompare), None)
                .unwrap();
            assert_eq!(resp.exit_code, local.exit_code);
        }
        other @ ViewRequest::Folder { .. } => panic!("{other:?}"),
    }
    // Folders: --filters travels in the patch; the legacy switch wins when both are given.
    write(&w, "l/x.c", b"1\n");
    write(&w, "r/x.c", b"2\n");
    let folder = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("size".into()),
        patch: FolderRulesPatch {
            name_filter: Some("*.c".into()),
            unix_group: Some(true),
            ..FolderRulesPatch::default()
        },
        ..FolderCompareRequest::default()
    };
    let words = ViewWords {
        display: Some("all".into()),
        folders: Some("files-only".into()),
        suppress_filters: true,
        expand_all: true,
        ..ViewWords::default()
    };
    let mut open = GuiOpenRequest::for_folder(&folder, &words, false);
    let resolved = service.resolve_open(&open).unwrap();
    assert_eq!(resolved.kind, ViewKind::FolderCompare);
    match service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .unwrap()
    {
        ViewRequest::Folder {
            request,
            display,
            expand_all,
        } => {
            assert_eq!(request.criteria.as_deref(), Some("size"));
            assert_eq!(request.patch.name_filter.as_deref(), Some("*.c"));
            assert_eq!(request.patch.unix_group, Some(true));
            assert_eq!(display.filter, FolderDisplayFilter::All);
            assert_eq!(display.folders, FolderMode::OnlyFiles);
            assert!(display.suppress_filters && expand_all);
        }
        other @ ViewRequest::Text { .. } => panic!("{other:?}"),
    }
    open.options.filters = Some("*.h".into());
    let resolved = service.resolve_open(&open).unwrap();
    let ViewRequest::Folder { request, .. } = service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .unwrap()
    else {
        panic!("folder view expected")
    };
    assert_eq!(request.patch.name_filter.as_deref(), Some("*.h"));
    // Relative specs, `-` and the wrong count are refused on the GUI side (§4.4.3 rule 4).
    for specs in [
        vec!["a.txt".to_owned(), "/b".to_owned()],
        vec!["-".to_owned(), "/b".to_owned()],
        vec!["/a".to_owned()],
    ] {
        assert!(
            service
                .open_view_request(ViewKind::TextCompare, &specs, &Default::default())
                .is_err(),
            "{specs:?}"
        );
    }
}

#[test]
fn bad_vocabulary_is_found_before_the_gui_is_contacted() {
    let tmp = common::short_tempdir();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/x", b"1\n");
    write(&w, "r/x", b"2\n");
    let a = write(&w, "a.txt", b"a\n");
    let b = write(&w, "b.txt", b"b\n");
    // No GUI is running, so reaching the connect step would be Ipc(NotRunning).
    let folder = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("bogus".into()),
        ..FolderCompareRequest::default()
    };
    let err = service
        .gui_open(
            &GuiOpenRequest::for_folder(&folder, &ViewWords::default(), false),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::Folder(_)), "{err:?}");
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
    let text = TextCompareRequest {
        left: os(&a),
        right: os(&b),
        format: Some("no-such-format".into()),
        ..TextCompareRequest::default()
    };
    let err = service
        .gui_open(
            &GuiOpenRequest::for_text(&text, &ViewWords::default(), false),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::Format(_)), "{err:?}");
    let words = ViewWords {
        display: Some("sometimes".into()),
        ..ViewWords::default()
    };
    let text = TextCompareRequest {
        left: os(&a),
        right: os(&b),
        ..TextCompareRequest::default()
    };
    assert!(
        service
            .gui_open(&GuiOpenRequest::for_text(&text, &words, false), None)
            .is_err()
    );
    // A rules file `-` can never reach the GUI.
    let mut open = GuiOpenRequest::for_text(&text, &ViewWords::default(), false);
    open.text_rules_file = Some("-".into());
    let err = service.gui_open(&open, None).unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn standard_input_is_read_lazily_and_once() {
    let tmp = tempfile::tempdir().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&reads);
    let source = StdinSource::new(Some("hint.py".into()), move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(b"x = 1\n".to_vec())
    });
    let service = service_with(
        tmp.path(),
        Some(source),
        Arc::new(common::UpperRunner::default()),
    );
    let w = tmp.path().join("work");
    let a = write(&w, "a.py", b"x = 1\n");
    // `format detect -` needs the name only.
    let detect = service.format_detect(&OsString::from("-")).unwrap();
    assert_eq!(detect.format.id.as_deref(), Some("python"));
    // A command without `-` never reads.
    service
        .quick(
            &QuickRequest {
                left: os(&a),
                right: os(&a),
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    // The first `-` reads; later ones reuse the content.
    for _ in 0..2 {
        let resp = service
            .quick(
                &QuickRequest {
                    left: OsString::from("-"),
                    right: os(&a),
                    ..QuickRequest::default()
                },
                &job(&service, JobKind::QuickCompare),
            )
            .unwrap();
        assert_eq!(resp.exit_code, ExitCode::BinarySame);
    }
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    // A failing read is an error of the command (exit 100), not a panic.
    let failing = service_with(
        &tmp.path().join("f"),
        Some(StdinSource::new(None, || {
            Err(std::io::Error::other("stdin is closed"))
        })),
        Arc::new(common::UpperRunner::default()),
    );
    let err = failing.parse_spec(&OsString::from("-")).unwrap_err();
    assert!(err.to_string().contains("stdin is closed"), "{err}");
    assert_eq!(err.exit_code(ApiOp::Quick), ExitCode::UnknownError);
}

#[test]
fn the_d23_close_rules_are_one_function() {
    let cases = [
        (ViewOutcome::NoVerdict, 0, "user", None, None),
        (ViewOutcome::LoadFailed, 100, "load_failed", None, None),
        (
            ViewOutcome::Text {
                result: ContentResult::UnimportantOnly,
                ignore_unimportant: false,
            },
            12,
            "user",
            Some(ContentKind::UnimportantOnly),
            None,
        ),
        (
            ViewOutcome::Text {
                result: ContentResult::UnimportantOnly,
                ignore_unimportant: true,
            },
            2,
            "user",
            Some(ContentKind::UnimportantOnly),
            None,
        ),
        (
            ViewOutcome::Folder {
                verdict: CompareVerdict::Incomplete,
            },
            100,
            "user",
            None,
            Some(CompareVerdict::Incomplete),
        ),
        (
            ViewOutcome::Folder {
                verdict: CompareVerdict::Differences,
            },
            13,
            "user",
            None,
            Some(CompareVerdict::Differences),
        ),
    ];
    for (outcome, code, reason, verdict, folder) in cases {
        let (result, exit) = outcome.close(CloseReason::User, false);
        assert_eq!(exit.code(), code, "{outcome:?}");
        assert_eq!(result.reason, reason, "{outcome:?}");
        assert_eq!(result.verdict_kind(), verdict);
        assert_eq!(result.folder_verdict_kind(), folder);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_file_pair_opens_from_its_specs() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    std::fs::create_dir_all(w.join("L")).unwrap();
    std::fs::create_dir_all(w.join("R")).unwrap();
    std::fs::write(w.join("L").join(name), b"one\n").unwrap();
    std::fs::write(w.join("R").join(name), b"two\n").unwrap();
    let left = service
        .parse_spec(w.join("L").join(name).as_os_str())
        .unwrap();
    let right = service
        .parse_spec(w.join("R").join(name).as_os_str())
        .unwrap();
    assert!(left.is_lossy());
    let resp = service
        .text_compare(
            &TextCompareRequest::for_specs(&left, &right),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert_eq!(resp.exit_code, ExitCode::RulesDifferent);
    assert!(matches!(left, Spec::Local(_)));
}

#[test]
fn a_second_instance_forwards_absolute_specs_without_waiting() {
    let tmp = common::short_tempdir();
    let service = service(tmp.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = start(&service, Arc::clone(&gui));
    let w = tmp.path().join("work");
    write(&w, "a.txt", b"a\n");
    write(&w, "b.txt", b"b\n");
    let args = parse_open_args(
        &[
            OsString::from("a.txt"),
            OsString::from("b.txt"),
            OsString::from("--wait"),
        ],
        Platform::HOST,
    )
    .unwrap();
    let report = service.forward_open(&args.request).unwrap();
    assert!(matches!(report.outcome, GuiOpenOutcome::Opened { .. }));
    assert_eq!(report.exit_code(), 0);
    let sent = gui.requests.lock().clone();
    let Some(Request::OpenView { specs, wait, .. }) = sent.last() else {
        panic!("{sent:?}")
    };
    assert!(!wait);
    assert!(
        specs.iter().all(|s| Path::new(s).is_absolute()),
        "{specs:?}"
    );
    drop(gui.waiters.lock().drain(..).collect::<Vec<_>>());
    server.shutdown().unwrap();
    // The launcher is never used for forwarding.
    let _unused = FakeLauncher::default();
}

#[test]
fn the_usage_text_covers_the_switch_table() {
    let usage = open_usage();
    for switch in [
        "--wait",
        "--no-launch",
        "--fv",
        "--title1",
        "--ro2",
        "--iu",
        "--stdin-name",
        "--solo",
    ] {
        assert!(usage.contains(switch), "{switch} missing:\n{usage}");
    }
    assert!(usage.contains("--readonly") && usage.contains("--automerge"));
}

#[test]
fn rules_file_errors_name_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let bad = write(&w, "bad.toml", b"[[[");
    let err = service
        .text_compare(
            &TextCompareRequest {
                left: os(&a),
                right: os(&a),
                rules_file: Some(os(&bad)),
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap_err();
    let report = err.report();
    assert_eq!(report.spec.as_deref(), Some(&*bad.to_string_lossy()));
    assert!(report.message.contains("bad.toml"), "{report:?}");
    assert_eq!(err.exit_code(ApiOp::TextCompare), ExitCode::UnknownError);
    write(&w, "l/x", b"1\n");
    write(&w, "r/x", b"2\n");
    let err = service
        .folder_prepare(&FolderCompareRequest {
            left: os(&w.join("l")),
            right: os(&w.join("r")),
            rules_file: Some(os(&bad)),
            ..FolderCompareRequest::default()
        })
        .unwrap_err();
    assert_eq!(err.report().spec.as_deref(), Some(&*bad.to_string_lossy()));
    assert_eq!(err.exit_code(ApiOp::FolderCompare), ExitCode::UnknownError);
}

#[test]
fn text_rules_without_rules_content_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/x", b"1\n");
    write(&w, "r/x", b"2\n");
    let prepared = service
        .folder_prepare(&FolderCompareRequest {
            left: os(&w.join("l")),
            right: os(&w.join("r")),
            criteria: Some("size crc".into()),
            text_rules_file: Some(os(&w.join("does-not-exist.toml"))),
            ..FolderCompareRequest::default()
        })
        .unwrap();
    assert_eq!(prepared.rules().criteria.content.method, ContentMethod::Crc);
    assert!(
        prepared
            .warnings()
            .iter()
            .any(|w| w.code == WarningCode::SwitchIgnored),
        "{:?}",
        prepared.warnings()
    );
}

#[test]
fn options_toml_edge_cases_are_not_silent() {
    use hikasa_session::options::parse_options;
    let err = parse_options("text_editing = 5\n", "options.toml").unwrap_err();
    assert!(err.to_string().contains("must be a table"), "{err}");
    let load = parse_options(
        "\"text_editing.number_of_context_lines\" = 9\n",
        "options.toml",
    )
    .unwrap();
    assert_eq!(
        load.options.context_lines, 3,
        "a quoted dotted key is not the option"
    );
    assert_eq!(load.warnings.len(), 1);
    assert_eq!(
        load.ignored_keys,
        vec!["\"text_editing.number_of_context_lines\"".to_owned()]
    );
    let err = parse_options("schema_version = -1\n", "options.toml").unwrap_err();
    assert!(err.to_string().contains("non-negative"), "{err}");
    assert!(matches!(err, hikasa_session::SessionError::Config { .. }));
    // The known keys still work in both spellings.
    let load = parse_options(
        "[text_editing]\nnumber_of_context_lines = 7\n[tweaks.file_operations]\ncontent_memory_budget_mib = 64\n",
        "options.toml",
    )
    .unwrap();
    assert_eq!(load.options.context_lines, 7);
    assert_eq!(load.options.content_memory_budget_mib, 64);
}

#[cfg(unix)]
#[test]
fn switch_values_that_are_not_utf8_are_applied() {
    use std::os::unix::ffi::OsStringExt;
    let mut raw = b"--config-dir=/data/hk-".to_vec();
    raw.push(0xFF);
    let parsed = parse_open_args(
        &[OsString::from_vec(raw.clone()), "a".into(), "b".into()],
        Platform::Linux,
    )
    .unwrap();
    let mut dir = b"/data/hk-".to_vec();
    dir.push(0xFF);
    assert_eq!(
        parsed
            .config_dir
            .as_deref()
            .map(|p| p.as_os_str().to_owned()),
        Some(OsString::from_vec(dir))
    );
    assert_eq!(
        parsed.request.specs,
        vec![OsString::from("a"), OsString::from("b")]
    );
    let mut title = b"--title1=".to_vec();
    title.push(0xFF);
    assert!(matches!(
        parse_open_args(&[OsString::from_vec(title), "a".into()], Platform::Linux),
        Err(ApiError::InvalidArgument(_))
    ));
}

#[test]
fn unknown_windows_slash_switches_are_errors_and_paths_stay_paths() {
    assert!(matches!(
        parse_open_args(
            &["/readonlyy".into(), "C:\\w\\a.txt".into()],
            Platform::Windows
        ),
        Err(ApiError::InvalidArgument(_))
    ));
    let parsed = parse_open_args(
        &["/ro".into(), "/dir/a.txt".into(), "/b.txt".into()],
        Platform::Windows,
    )
    .unwrap();
    assert!(parsed.request.options.read_only.all);
    assert_eq!(parsed.request.specs.len(), 2);
    // On Unix a leading slash is always a path.
    let parsed = parse_open_args(&["/readonlyy".into(), "/b".into()], Platform::Linux).unwrap();
    assert_eq!(parsed.request.specs.len(), 2);
}

#[test]
fn empty_line_ranges_select_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let file = write(&tmp.path().join("work"), "a.c", b"int a;\nint b;\nint c;\n");
    #[allow(clippy::reversed_empty_ranges)]
    let ranges = [1u32..=0, 0..=0, 3..=2, 9..=12];
    for range in ranges {
        let dto = service
            .format_tokens(
                &FormatTokensRequest {
                    file: os(&file),
                    lines: Some(range.clone()),
                    ..FormatTokensRequest::default()
                },
                &job(&service, JobKind::Report),
            )
            .unwrap();
        assert!(dto.lines.is_empty(), "{range:?} selected {:?}", dto.lines);
    }
    let dto = service
        .format_tokens(
            &FormatTokensRequest {
                file: os(&file),
                lines: Some(hikasa_session::api::parse_line_range("2-9").unwrap()),
                ..FormatTokensRequest::default()
            },
            &job(&service, JobKind::Report),
        )
        .unwrap();
    assert_eq!(
        dto.lines.iter().map(|l| l.line).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[test]
fn envelope_helpers_for_raw_codes_and_usage_errors() {
    let raw = JsonEnvelope::success_raw("hikasa.gui_open.v1", 250, (), vec![]);
    assert_eq!(raw.exit_code, 250);
    let usage = JsonEnvelope::usage_error("unexpected argument '--frob'");
    let value = serde_json::to_value(&usage).unwrap();
    assert_eq!(value["schema"], USAGE_ERROR_SCHEMA);
    assert_eq!(value["exit_code"], 100);
    assert_eq!(value["ok"], false);
    assert_eq!(value["result"], serde_json::Value::Null);
    assert_eq!(value["error"]["kind"], "invalid_input");
}

#[test]
fn grammar_dependent_report_paths() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/lines.toml",
        b"schema_version = 1\nid = \"lines\"\nname = \"Lines\"\nkind = \"text\"\nmask = \"*.lst\"\n\n[text]\nlines_independent = true\n",
    );
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let render = |resp: &hikasa_session::api::TextCompareResponse, opts: TextReportOptions| {
        let mut out = Vec::new();
        service
            .render_text_with(resp, &opts, &ReportSettings::default(), &mut out)
            .unwrap();
        String::from_utf8(out).unwrap()
    };
    let compare = |a: &Path, b: &Path| {
        service
            .text_compare(
                &TextCompareRequest {
                    left: os(a),
                    right: os(b),
                    ..TextCompareRequest::default()
                },
                &job(&service, JobKind::TextCompare),
                None,
            )
            .unwrap()
    };
    // JavaScript has tab size 2: a TAB in the side-by-side text expands to two columns.
    let a = write(&w, "a.js", b"\tx = 1;\n");
    let b = write(&w, "b.js", b"\tx = 2;\n");
    let resp = compare(&a, &b);
    let sbs = render(
        &resp,
        TextReportOptions {
            layout: TextLayout::SideBySide,
            ..TextReportOptions::default()
        },
    );
    assert!(sbs.contains("  x = 1;"), "{sbs}");
    assert!(!sbs.contains("    x = 1;"), "tab size 2, not 4: {sbs}");
    // A lines_independent user format: one normal-diff command per differing row.
    let a = write(&w, "a.lst", b"a\nb\nc\n");
    let b = write(&w, "b.lst", b"A\nB\nc\n");
    let resp = compare(&a, &b);
    assert_eq!(resp.format.id.as_ref().map(|id| id.as_str()), Some("lines"));
    let normal = render(
        &resp,
        TextReportOptions {
            layout: TextLayout::Patch(PatchStyle::Normal),
            ..TextReportOptions::default()
        },
    );
    assert!(normal.contains("1d0\n< a\n2d0\n< b\n"), "{normal}");
    assert!(!normal.contains("1,2d0"), "one command per row: {normal}");
    // The RPT-004 rules line names the C grammar.
    let a = write(&w, "a.c", b"int a;\n");
    let b = write(&w, "b.c", b"int b;\n");
    let resp = compare(&a, &b);
    let header = render(&resp, TextReportOptions::default());
    assert!(
        header.contains("Rules: format: C, C++, C#, ObjC Source;"),
        "{header}"
    );
    // format_mismatch reaches the response warnings (JSON envelope, stderr), not the report header (§1.7.3
    // amendment).
    let py = write(&w, "b.py", b"int b;\n");
    let resp = compare(&a, &py);
    assert!(
        resp.warnings
            .iter()
            .any(|w| w.code == WarningCode::FormatMismatch)
    );
}

#[test]
fn the_title_reaches_folder_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/x", b"1\n");
    write(&w, "r/x", b"2\n");
    let resp = service
        .folder_compare(
            &FolderCompareRequest {
                left: os(&w.join("l")),
                right: os(&w.join("r")),
                ..FolderCompareRequest::default()
            },
            &job(&service, JobKind::FolderCompare),
            &NullObserver,
        )
        .unwrap();
    for layout in [FolderLayout::SideBySide, FolderLayout::Summary] {
        let mut out = Vec::new();
        service
            .render_folder_with(
                &resp,
                &FolderReportOptions {
                    layout,
                    ..FolderReportOptions::default()
                },
                &ReportSettings {
                    title: Some("Nightly diff".into()),
                    ..ReportSettings::default()
                },
                &mut out,
            )
            .unwrap();
        assert!(String::from_utf8(out).unwrap().contains("Nightly diff"));
    }
}

#[test]
fn a_user_format_from_a_newer_version_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/new.toml",
        b"schema_version = 2\nid = \"new\"\nname = \"New\"\nkind = \"text\"\nmask = \"*.new\"\n",
    );
    let service = service(tmp.path());
    let skipped: Vec<_> = service
        .startup_warnings()
        .iter()
        .filter(|w| w.code == WarningCode::UserFormatSkipped)
        .collect();
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].message.contains("new.toml"), "{skipped:?}");
    assert!(skipped[0].message.contains('2'), "{skipped:?}");
    assert!(
        service.registry().find("c-cpp").is_some(),
        "the built-ins stay"
    );
    assert!(service.registry().find("new").is_none());
}
