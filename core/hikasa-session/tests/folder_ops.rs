//! M2 W6b acceptance through the Service: A6.21 (Disable editing, D-184), A6.22's session half (Exclude, FC-052), A6.23 (Open
//! With, D-186, on `RecordingLauncher`), A6.24 (the log model, D-188), T11: the same plan through the Service and the CLI.

mod common;

use std::ffi::OsString;
use std::sync::Arc;

use common::{facts, fixed_clock, job, os, write};
use hikasa_core::{ContentMethod, ErrorKind, ExitCode, JobKind, RelPath, Side, WarningCode};
use hikasa_folder::ops::{
    FolderOp, OpPlan, OpVerb, ReadOnlySides, Recycle, RenameMask, TouchFrom,
};
use hikasa_folder::select::{ExcludeWhat, SideSet};
use hikasa_folder::NullObserver;
use hikasa_session::api::folder_ops::{item_line, summary_text};
use hikasa_session::api::sessions::ExcludeMasksDto;
use hikasa_session::api::{
    ApiOp, ContentsKind, ContentsRequest, FolderCompareRequest, FolderOpOutcome, FolderOpRequest,
    OpOverrides, OpRequest, OpenTarget, OpenWithRequest, PreparedFolderCompare, SelectRequest,
};
use hikasa_session::launcher::{OpenCommand, Recorded, RecordingLauncher};
use hikasa_session::log::{LOG_DETAILS_MAX, LogFilter, LogKind, op_log_entries};
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType};
use hikasa_session::{ApiError, Service, ServiceInit};
use hikasa_vfs::ops::OverwritePolicy;

fn service_on(root: &std::path::Path, launcher: Arc<RecordingLauncher>) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(2),
        launcher,
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

fn service(root: &std::path::Path) -> Service {
    service_on(root, Arc::new(RecordingLauncher::default()))
}

fn prepared(service: &Service, root: &std::path::Path) -> PreparedFolderCompare {
    std::fs::create_dir_all(root.join("l")).unwrap();
    std::fs::create_dir_all(root.join("r")).unwrap();
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(service, JobKind::FolderCompare);
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    prepared
}

fn every_row() -> SelectRequest {
    SelectRequest {
        masks: vec!["all".to_owned()],
        ..SelectRequest::default()
    }
}

fn request(op: OpRequest, read_only: ReadOnlySides) -> FolderOpRequest {
    FolderOpRequest {
        op,
        select: every_row(),
        read_only,
        overrides: OpOverrides::default(),
    }
}

fn plan_of(
    service: &Service,
    prepared: &PreparedFolderCompare,
    req: &FolderOpRequest,
) -> Result<OpPlan, ApiError> {
    let control = job(service, JobKind::FolderOp);
    service.folder_op_plan(prepared.model(), req, &control)
}

const LEFT_ONLY: ReadOnlySides = ReadOnlySides {
    left: true,
    right: false,
};
const RIGHT_ONLY: ReadOnlySides = ReadOnlySides {
    left: false,
    right: true,
};

fn copy(to: Side) -> OpRequest {
    OpRequest::Op(FolderOp::Copy {
        to,
        overwrite: OverwritePolicy::Always,
    })
}

fn move_to(to: Side) -> OpRequest {
    OpRequest::Op(FolderOp::Move {
        to,
        overwrite: OverwritePolicy::Always,
    })
}

fn tree(root: &std::path::Path) {
    write(root, "l/a.txt", b"one");
    write(root, "l/b.txt", b"left");
    write(root, "r/b.txt", b"right side");
}

#[test]
fn a6_21_every_row_of_the_table_is_refused_before_a_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let prepared = prepared(&service, root);
    let cases = [
        (copy(Side::Right), RIGHT_ONLY, Side::Right),
        (copy(Side::Left), LEFT_ONLY, Side::Left),
        (move_to(Side::Right), RIGHT_ONLY, Side::Right),
        (move_to(Side::Right), LEFT_ONLY, Side::Left),
        (OpRequest::Op(FolderOp::Exchange), LEFT_ONLY, Side::Left),
        (OpRequest::Op(FolderOp::Exchange), RIGHT_ONLY, Side::Right),
        (
            OpRequest::Op(FolderOp::Delete {
                sides: SideSet::LEFT,
                recycle: Recycle::default(),
            }),
            LEFT_ONLY,
            Side::Left,
        ),
        (
            OpRequest::Op(FolderOp::Rename {
                sides: SideSet::RIGHT,
                mask: RenameMask::Dos("*.bak".to_owned()),
            }),
            RIGHT_ONLY,
            Side::Right,
        ),
        (
            OpRequest::TouchNow {
                sides: SideSet::all(),
            },
            RIGHT_ONLY,
            Side::Right,
        ),
        (
            OpRequest::Op(FolderOp::Touch {
                sides: SideSet::RIGHT,
                from: TouchFrom::OtherSide { to: Side::Right },
            }),
            RIGHT_ONLY,
            Side::Right,
        ),
        (
            OpRequest::MkdirIn {
                sides: SideSet::LEFT,
                parent: String::new(),
                name: "x".to_owned(),
                folder: false,
            },
            LEFT_ONLY,
            Side::Left,
        ),
        (
            OpRequest::MoveTo {
                sides: SideSet::LEFT,
                target: os(&root.join("out")),
                structure: Default::default(),
                overwrite: OverwritePolicy::Fail,
            },
            LEFT_ONLY,
            Side::Left,
        ),
    ];
    for (op, read_only, side) in cases {
        let err = plan_of(&service, &prepared, &request(op.clone(), read_only)).unwrap_err();
        assert!(
            matches!(err, ApiError::ReadOnlySide { side: found } if found == side),
            "{op:?}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(err.exit_code(ApiOp::FolderOp), ExitCode::UnknownError);
    }
    assert!(!root.join("out").exists());
}

#[test]
fn a6_21_what_writes_the_other_side_goes_on() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let prepared = prepared(&service, root);
    assert!(plan_of(&service, &prepared, &request(copy(Side::Right), LEFT_ONLY)).is_ok());
    assert!(plan_of(&service, &prepared, &request(copy(Side::Left), RIGHT_ONLY)).is_ok());
}

#[test]
fn a6_21_copyto_out_of_a_read_only_side_is_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let prepared = prepared(&service, root);
    let op = OpRequest::CopyTo {
        sides: SideSet::LEFT,
        target: os(&root.join("out")),
        structure: Default::default(),
        overwrite: OverwritePolicy::Fail,
    };
    let plan = plan_of(&service, &prepared, &request(op, LEFT_ONLY)).unwrap();
    assert!(!plan.plan.items.is_empty());
}

#[test]
fn a6_21_ignored_and_exclude_change_a_session_not_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    let service = service(root);
    let prepared = prepared(&service, root);
    let select = every_row();
    assert!(
        service
            .folder_exclude_masks(prepared.model(), &select, ExcludeWhat::Type)
            .is_ok()
    );
    let rows = service.folder_selected_rows(prepared.model(), &select).unwrap();
    assert!(!service.folder_set_ignored(prepared.model(), &rows, true).is_empty());
}

#[test]
fn a6_21_a_session_keeps_its_sides_and_swaps_them() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let reference = SessionRef::parse("ro").unwrap();
    let mut session = service.session_defaults(SessionType::FolderCompare).unwrap();
    session.specs.left = root.join("l").display().to_string();
    session.specs.right = root.join("r").display().to_string();
    session.specs.read_only_left = true;
    service.session_save(&reference, &session, false).unwrap();
    let sides = service.session_read_only(&reference).unwrap();
    assert_eq!(sides, LEFT_ONLY);
    let mut loaded: Session = service.session_load(&reference).unwrap().value;
    assert!(loaded.specs.read_only_left);
    let before = loaded.clone();
    loaded.swap_sides();
    assert!(loaded.specs.read_only_right && !loaded.specs.read_only_left);
    assert!(hikasa_session::sessions::is_dirty(&before, &loaded));
}

#[test]
fn a6_16_the_strict_machine_asks_delete_whatever_the_option_says() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    tree(root);
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("config/options.toml"),
        "schema_version = 1\n[file_operations]\nconfirm_delete = false\n",
    )
    .unwrap();
    let service = service(root);
    let prepared = prepared(&service, root);
    let op = OpRequest::Op(FolderOp::Delete {
        sides: SideSet::LEFT,
        recycle: Recycle::default(),
    });
    let plan = plan_of(&service, &prepared, &request(op, ReadOnlySides::default())).unwrap();
    assert!(service.folder_confirmation(&plan, None).next().is_none());
    assert!(service.folder_confirmation_strict(&plan, None).next().is_some());
}

#[test]
fn a6_15_the_default_honours_filters_and_the_option_keeps_working() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a/x.txt", b"1");
    write(root, "l/a/y.bak", b"2");
    let service_of = |root: &std::path::Path| service(root);
    let make = |service: &Service| {
        std::fs::create_dir_all(root.join("r")).unwrap();
        let request = FolderCompareRequest {
            left: os(&root.join("l")),
            right: os(&root.join("r")),
            patch: hikasa_folder::FolderRulesPatch {
                name_filter: Some("-*.bak".to_owned()),
                ..Default::default()
            },
            ..FolderCompareRequest::default()
        };
        let control = job(service, JobKind::FolderCompare);
        let prepared = service.folder_prepare_in(&request, &control).unwrap();
        service
            .folder_run(&prepared, &control, &NullObserver)
            .unwrap();
        prepared
    };
    let targets = |plan: &OpPlan| -> Vec<String> {
        plan.plan.items.iter().map(|item| item.target.file_name().unwrap()).collect()
    };
    let service = service_of(root);
    let model = make(&service);
    let op = copy(Side::Right);
    let plan = plan_of(&service, &model, &request(op.clone(), ReadOnlySides::default())).unwrap();
    assert_eq!(targets(&plan), ["a", "x.txt"]);
    drop(service);
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("config/options.toml"),
        "schema_version = 1\n[file_operations]\ninclude_hidden_by_default = true\n",
    )
    .unwrap();
    let service = service_of(root);
    let model = make(&service);
    let plan = plan_of(&service, &model, &request(op, ReadOnlySides::default())).unwrap();
    assert_eq!(targets(&plan), ["a", "x.txt", "y.bak"]);
}

#[test]
fn a6_22_exclude_rewrites_the_rules_and_a_rerun_hides_the_item() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a/x.txt", b"1");
    write(root, "l/keep.txt", b"1");
    write(root, "r/keep.txt", b"1");
    let service = service(root);
    let reference = SessionRef::parse("ex").unwrap();
    let mut session = service.session_defaults(SessionType::FolderCompare).unwrap();
    session.specs.left = root.join("l").display().to_string();
    session.specs.right = root.join("r").display().to_string();
    service.session_save(&reference, &session, false).unwrap();
    let prepared = prepared(&service, root);
    let select = SelectRequest {
        paths: vec![("a/x.txt".to_owned(), SideSet::LEFT)],
        ..SelectRequest::default()
    };
    let (files, folders) = service
        .folder_exclude_masks(prepared.model(), &select, ExcludeWhat::Items)
        .unwrap();
    assert_eq!(files, ["a\\x.txt"]);
    assert!(folders.is_empty());
    let masks = ExcludeMasksDto { files, folders };
    let dto = service.session_add_excludes(&reference, &masks, false).unwrap();
    assert_eq!(dto.masks.as_ref(), Some(&masks));
    // Nothing is added twice.
    service.session_add_excludes(&reference, &masks, false).unwrap();
    let saved = service.session_load(&reference).unwrap().value;
    let SessionRules::Folder(rules) = &saved.rules else {
        panic!("folder rules");
    };
    assert_eq!(rules.filters.names.exclude_files, ["a\\x.txt"]);
    // Rules are immutable in a model: the view re-runs, and the item is gone.
    let again = hikasa_session::api::SessionRunRequest::Folder(FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        base_rules: Some(rules.clone()),
        ..FolderCompareRequest::default()
    });
    let hikasa_session::api::SessionRunRequest::Folder(again) = again else {
        panic!("folder");
    };
    let control = job(&service, JobKind::FolderCompare);
    let rerun = service.folder_prepare_in(&again, &control).unwrap();
    service.folder_run(&rerun, &control, &NullObserver).unwrap();
    let read = rerun.model().read();
    let hidden = read
        .node_at(&RelPath::parse("a/x.txt").unwrap())
        .and_then(|node| read.node(node))
        .is_none_or(|view| view.left.is_none_or(|entry| entry.filtered));
    assert!(hidden);
}

#[test]
fn a6_22_a_locked_session_needs_force() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let reference = SessionRef::parse("locked").unwrap();
    let session = service.session_defaults(SessionType::FolderCompare).unwrap();
    service.session_save(&reference, &session, false).unwrap();
    service.session_lock(&reference, true).unwrap();
    let masks = ExcludeMasksDto {
        files: vec!["*.tmp".to_owned()],
        folders: Vec::new(),
    };
    assert!(service.session_add_excludes(&reference, &masks, false).is_err());
    assert!(service.session_add_excludes(&reference, &masks, true).is_ok());
    assert!(service.session_set_ignored(&reference, &["x".to_owned()], false).is_err());
    assert!(service.session_set_ignored(&reference, &["x".to_owned()], true).is_ok());
}

fn openwith(root: &std::path::Path, text: &str) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("config/openwith.toml"),
        format!("schema_version = 1\n{text}"),
    )
    .unwrap();
}

fn open_items(root: &std::path::Path, names: &[&str]) -> Vec<(hikasa_core::Spec, hikasa_core::Spec)> {
    let spec = |path: std::path::PathBuf| {
        hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path).unwrap())
    };
    names
        .iter()
        .map(|name| (spec(root.join("l").join(name)), spec(root.join("l"))))
        .collect()
}

fn with_launcher(root: &std::path::Path, answers: Vec<Option<i32>>) -> (Service, Arc<RecordingLauncher>) {
    let launcher = Arc::new(RecordingLauncher::with_answers(answers));
    (service_on(root, launcher.clone()), launcher)
}

fn arg_texts(command: &OpenCommand) -> Vec<String> {
    command.args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

#[test]
fn a6_23_per_item_and_per_pair() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"1");
    write(root, "l/b.txt", b"2");
    openwith(
        root,
        "[[app]]\ndescription = \"one\"\ncommand = \"tool %n\"\n\n[[app]]\ndescription = \"two\"\ncommand = \"diff %n %n2\"\n",
    );
    let (service, _) = with_launcher(root, Vec::new());
    let control = job(&service, JobKind::FolderOp);
    let request = |target| OpenWithRequest {
        target,
        items: open_items(root, &["a.txt", "b.txt"]),
        line: None,
    };
    let per_item = service
        .open_with_commands(&request(OpenTarget::Named("one".to_owned())), &control)
        .unwrap();
    assert_eq!(per_item.len(), 2);
    assert_eq!(per_item[0].program, OsString::from("tool"));
    assert_eq!(arg_texts(&per_item[1]), ["b.txt"]);
    let per_pair = service
        .open_with_commands(&request(OpenTarget::App(1)), &control)
        .unwrap();
    assert_eq!(per_pair.len(), 1);
    assert_eq!(arg_texts(&per_pair[0]), ["a.txt", "b.txt"]);
    // A `%…2` without a second item is `invalid_input` (D-76).
    let one = OpenWithRequest {
        target: OpenTarget::App(1),
        items: open_items(root, &["a.txt"]),
        line: None,
    };
    let err = service.open_with_commands(&one, &control).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(
        service
            .open_with_commands(
                &OpenWithRequest {
                    target: OpenTarget::App(7),
                    items: open_items(root, &["a.txt"]),
                    line: None,
                },
                &control
            )
            .is_err()
    );
}

#[test]
fn a6_23_a_folder_app_and_accepts_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/d/x", b"1");
    write(root, "l/f", b"1");
    openwith(
        root,
        "[[app]]\ndescription = \"files\"\ncommand = \"a %n\"\n\n[[app]]\ndescription = \"folders\"\ncommand = \"b %n\"\naccepts_files = false\naccepts_folders = true\n",
    );
    let (service, _) = with_launcher(root, Vec::new());
    let control = job(&service, JobKind::FolderOp);
    let open = |app: usize, name: &str| {
        service.open_with_commands(
            &OpenWithRequest {
                target: OpenTarget::App(app),
                items: open_items(root, &[name]),
                line: None,
            },
            &control,
        )
    };
    assert!(open(0, "f").is_ok());
    assert_eq!(open(0, "d").unwrap_err().kind(), ErrorKind::InvalidInput);
    assert!(open(1, "d").is_ok());
    assert_eq!(open(1, "f").unwrap_err().kind(), ErrorKind::InvalidInput);
}

#[test]
fn a6_23_a_member_is_extracted_and_described_by_its_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("inner.txt", options).unwrap();
    std::io::Write::write_all(&mut zip, b"member").unwrap();
    write(root, "l/a.zip", &zip.finish().unwrap().into_inner());
    openwith(root, "[[app]]\ndescription = \"one\"\ncommand = \"tool %f\"\n");
    let (service, _) = with_launcher(root, Vec::new());
    let archive = hikasa_core::Spec::Local(
        hikasa_core::LocalPath::new(root.join("l/a.zip")).unwrap(),
    );
    let member = hikasa_core::Spec::archive_member(archive.clone(), "inner.txt");
    let control = job(&service, JobKind::FolderOp);
    let commands = service
        .open_with_commands(
            &OpenWithRequest {
                target: OpenTarget::App(0),
                items: vec![(member, archive)],
                line: None,
            },
            &control,
        )
        .unwrap();
    let path = std::path::PathBuf::from(&commands[0].args[0]);
    assert_eq!(path.file_name().unwrap(), "inner.txt");
    assert_eq!(std::fs::read(&path).unwrap(), b"member");
}

#[test]
fn a6_23_wait_for_previous_keeps_the_order_and_all_at_once_starts_each() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (service, launcher) = with_launcher(root, Vec::new());
    let command = |name: &str| OpenCommand {
        program: OsString::from(name),
        args: Vec::new(),
        cwd: root.to_path_buf(),
    };
    let control = job(&service, JobKind::FolderOp);
    let commands = [command("first"), command("second"), command("third")];
    service.open_with_run(&commands, true, &control).unwrap();
    let order: Vec<_> = launcher
        .calls()
        .into_iter()
        .map(|call| match call {
            Recorded::Run(command) => command.program,
            Recorded::Associated(path) => path.into_os_string(),
        })
        .collect();
    assert_eq!(order, ["first", "second", "third"]);
    service.open_with_run(&commands, false, &control).unwrap();
    assert_eq!(launcher.calls().len(), 6);
}

#[test]
fn a6_23_a_program_that_cannot_start_and_a_programs_own_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (service, _) = with_launcher(root, vec![Some(3), None]);
    let command = OpenCommand {
        program: OsString::from("tool"),
        args: Vec::new(),
        cwd: root.to_path_buf(),
    };
    let control = job(&service, JobKind::FolderOp);
    // Exit code 3 is the program's.
    assert!(service.open_with_run(std::slice::from_ref(&command), true, &control).is_ok());
    let err = service
        .open_with_run(std::slice::from_ref(&command), true, &control)
        .unwrap_err();
    assert_eq!(err.exit_code(ApiOp::FolderOp), ExitCode::UnknownError);
}

#[test]
fn a6_23_associated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = write(root, "l/a.txt", b"1");
    let (service, launcher) = with_launcher(root, Vec::new());
    let spec = hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path.clone()).unwrap());
    let control = job(&service, JobKind::FolderOp);
    service.open_associated(&spec, &control).unwrap();
    assert_eq!(launcher.calls(), [Recorded::Associated(path)]);
}

#[test]
fn a6_23_no_launcher_is_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = write(root, "l/a.txt", b"1");
    let service = common::service(root);
    let spec = hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path).unwrap());
    let control = job(&service, JobKind::FolderOp);
    let err = service.open_associated(&spec, &control).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

fn outcome(service: &Service, prepared: &PreparedFolderCompare, op: OpRequest) -> FolderOpOutcome {
    let plan = plan_of(service, prepared, &request(op, ReadOnlySides::default())).unwrap();
    let control = job(service, JobKind::FolderOp);
    service.folder_op_execute(plan, &control).unwrap()
}

fn time() -> jiff::Zoned {
    "2026-10-30T14:03:07+01:00[+01:00]".parse().unwrap()
}

#[test]
fn a6_24_golden_lines_and_the_summary_of_one_function() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"one");
    let service = service(root);
    let prepared = prepared(&service, root);
    let done = outcome(&service, &prepared, copy(Side::Right));
    let filter = LogFilter {
        details: true,
        ..LogFilter::default()
    };
    let seconds = Some(std::time::Duration::from_millis(1500));
    let entries = op_log_entries(&done, &time(), "tab", seconds, &filter);
    assert_eq!(entries[0].kind, LogKind::Summary);
    assert_eq!(entries[0].text, "copied 1 of 1 items (0 skipped, 0 failed), 3 bytes, 1.5 s");
    assert_eq!(entries[0].source, "tab");
    assert_eq!(entries[0].time, time());
    assert_eq!(entries[1].kind, LogKind::Detail);
    let line = item_line(&done.dto().items[0], false);
    assert_eq!(entries[1].text, line);
    assert!(line.starts_with("R copy "), "{line}");
    // The human summary and the log's are one text.
    assert_eq!(
        summary_text(OpVerb::Copy, false, &done.report.summary, Some(1.5)),
        entries[0].text
    );
    let human = hikasa_session::api::human::folder_op_lines(&done.dto(), OpVerb::Copy);
    assert_eq!(
        human.last().unwrap(),
        &summary_text(OpVerb::Copy, false, &done.report.summary, None)
    );
    let sync = hikasa_session::log::sync_log_entries(
        &hikasa_session::api::FolderSyncOutcome { op: done },
        &time(),
        "tab",
        None,
        &LogFilter::default(),
    );
    assert_eq!(sync[0].text, "copied 1 of 1 items (0 skipped, 0 failed), 3 bytes");
}

#[test]
fn a6_24_filters_and_failures_always() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"one");
    write(root, "l/b.txt", b"two");
    let service = service(root);
    let prepared = prepared(&service, root);
    let plan = plan_of(&service, &prepared, &request(copy(Side::Right), ReadOnlySides::default()))
        .unwrap();
    // The right side changes under the plan: one item fails.
    write(root, "r/a.txt", b"in the way");
    let control = job(&service, JobKind::FolderOp);
    let done = service.folder_op_execute(plan, &control).unwrap();
    assert_eq!(done.report.summary.failed, 1);
    assert_eq!(done.exit_code(), ExitCode::UnknownError);
    let quiet = LogFilter {
        summaries: false,
        details: false,
        warnings: false,
        ..LogFilter::default()
    };
    let entries = op_log_entries(&done, &time(), "tab", None, &quiet);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, LogKind::Error);
    assert!(entries[0].text.starts_with("failed copy "), "{}", entries[0].text);
    assert!(entries[0].text.contains("changed since the plan was made"));
}

#[test]
fn a6_24_at_most_ten_thousand_details_then_a_count() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"one");
    let service = service(root);
    let prepared = prepared(&service, root);
    let mut done = outcome(&service, &prepared, copy(Side::Right));
    let item = done.report.plan.items[0].clone();
    let result = done.report.results[0].clone();
    let meta = done.meta[0].clone();
    done.report.plan.items = vec![item; LOG_DETAILS_MAX + 7];
    done.report.results = vec![result; LOG_DETAILS_MAX + 7];
    done.meta = vec![meta; LOG_DETAILS_MAX + 7];
    let filter = LogFilter {
        details: true,
        ..LogFilter::default()
    };
    let entries = op_log_entries(&done, &time(), "tab", None, &filter);
    let details = entries.iter().filter(|e| e.kind == LogKind::Detail).count();
    assert_eq!(details, LOG_DETAILS_MAX + 1);
    assert_eq!(entries.last().unwrap().text, "\u{2026} and 7 more");
}

#[test]
fn a6_24_warnings_keep_their_code() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"one");
    let service = service(root);
    let prepared = prepared(&service, root);
    let mut done = outcome(&service, &prepared, copy(Side::Right));
    done.warnings.push(hikasa_core::Warning {
        code: WarningCode::TrashFallback,
        message: "text".to_owned(),
        spec: None,
    });
    let entries = op_log_entries(&done, &time(), "tab", None, &LogFilter::default());
    let warned: Vec<_> = entries.iter().filter(|e| e.kind == LogKind::Warning).collect();
    assert_eq!(warned[0].text, "[trash_fallback] text");
    let without = LogFilter {
        warnings: false,
        ..LogFilter::default()
    };
    let entries = op_log_entries(&done, &time(), "tab", None, &without);
    assert!(entries.iter().all(|e| e.kind != LogKind::Warning));
}

#[test]
fn a6_25_an_expanded_rename_keeps_meta_and_items_in_step() {
    // `execute_with` reports where an item grew, and the outcome follows.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"one");
    let service = service(root);
    let prepared = prepared(&service, root);
    let done = outcome(&service, &prepared, move_to(Side::Right));
    assert_eq!(done.meta.len(), done.report.plan.items.len());
    assert!(root.join("r/a.txt").exists());
    assert!(!root.join("l/a.txt").exists());
}

#[test]
fn a6_12_contents_through_the_service() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"same");
    write(root, "r/a.txt", b"same");
    let service = service(root);
    let prepared = prepared(&service, root);
    let control = job(&service, JobKind::FolderOp);
    let compare = ContentsRequest {
        select: every_row(),
        kind: ContentsKind::Compare(ContentMethod::Binary),
        text_rules: None,
    };
    let dto = service.folder_contents(prepared.model(), &compare, &control).unwrap();
    assert_eq!((dto.op, dto.method), ("compare", Some("binary")));
    assert_eq!(dto.counts.binary_same, 1);
    let crc = ContentsRequest {
        select: every_row(),
        kind: ContentsKind::Crc(SideSet::all()),
        text_rules: None,
    };
    let dto = service.folder_contents(prepared.model(), &crc, &control).unwrap();
    assert_eq!(dto.items.len(), 2);
    assert_eq!(dto.items[0].crc32, dto.items[1].crc32);
    assert!(dto.ok());
}
