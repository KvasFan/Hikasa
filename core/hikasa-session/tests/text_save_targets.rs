//! M2 W10b acceptance A10.17 (D-275, D-277, D-278; contract §1.10.14 *Targets*, *Backups*) through the Service, on disk:
//! a symbolic link followed by a save (the link stays a link), a FIFO refused (Unix), Save As moving the document,
//! `--savetarget` on a Text Edit document and on the right pane of a Text Compare (the left one saves in place), every
//! `[backups] naming` style once and the `folder` style, `--nobackups`, a backup that fails failing its item; and
//! A10.32: a file whose name is not UTF-8 saved (Linux), Windows drive, UNC and `\\?\` targets of Save As and
//! `--savetarget` as pure tables on every platform.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use common::{job, os, service, write};
use hikasa_core::{ExitCode, JobKind, PathStyle, Side, Spec, SpecContext};
use hikasa_session::Service;
use hikasa_session::api::{
    LiveOptions, LiveTextView, SaveAnswers, TextCompareRequest, TextEditRequest, apply_save_report,
    save_exit_code, save_target_spec,
};
use hikasa_text::save::{
    SaveMode, SavePlan, SaveQuestion, SaveReport, SaveRequest, UnmappablePolicy,
};
use hikasa_text::{EditRequest, EditableDocument, GroupHint, TextPos};
use hikasa_vfs::ops::ItemStatus;

// ---- helpers -----------------------------------------------------------------------------------------------------

fn open_with(service: &Service, left: &Path, right: &Path, opts: LiveOptions) -> LiveTextView {
    let req = TextCompareRequest {
        left: os(left),
        right: os(right),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live(&req, opts, &job(service, JobKind::TextCompare), None)
        .unwrap()
}

fn open(service: &Service, left: &Path, right: &Path) -> LiveTextView {
    open_with(service, left, right, LiveOptions::default())
}

fn type_x(doc: &mut EditableDocument) {
    let at = TextPos::new(0, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: "X".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

fn plan(service: &Service, docs: &[(Option<Side>, &EditableDocument, &SaveRequest)]) -> SavePlan {
    service
        .text_save_plan(docs, &job(service, JobKind::TextSave))
        .unwrap()
}

/// Answers every question yes (`--yes`, `--force`) and executes.
fn run(service: &Service, plan: SavePlan, docs: &[&EditableDocument]) -> SaveReport {
    let driver = service.save_confirmation(
        &plan,
        SaveAnswers {
            yes: true,
            force: true,
            ..SaveAnswers::default()
        },
    );
    let plan = driver.finish(plan).unwrap();
    service
        .text_save_execute(plan, docs, &job(service, JobKind::TextSave))
        .unwrap()
}

/// An in-place save with the settings' backups.
fn in_place(service: &Service) -> SaveRequest {
    SaveRequest::in_place(
        service.backup_policy_for_save(false).unwrap(),
        UnmappablePolicy::Ask,
    )
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The service's spec of `path`.
fn spec(service: &Service, path: &Path) -> Spec {
    service.parse_spec(path.as_os_str()).unwrap()
}

// ---- links and special files -------------------------------------------------------------------------------------

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> bool {
    // Needs developer mode or the privilege; without it the test is skipped with the reason.
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn a10_17_a_link_is_followed_and_stays_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let real = write(&w, "real.txt", b"one\ntwo\n");
    let link = w.join("link.txt");
    if !symlink(Path::new("real.txt"), &link) {
        eprintln!(
            "skipped: this process cannot create symbolic links (Windows without developer mode)"
        );
        return;
    }
    let other = write(&w, "other.txt", b"one\n");
    let mut view = open(&service, &other, &link);
    type_x(&mut view.docs.right);
    let req = in_place(&service);
    let plan = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    let item = &plan.items[0];
    assert_eq!(item.target.canonical(), spec(&service, &real).canonical());
    assert_eq!(
        item.via_link.as_ref().map(Spec::canonical),
        Some(spec(&service, &link).canonical())
    );
    assert!(
        plan.human_lines()[0].contains(", via "),
        "{:?}",
        plan.human_lines()
    );
    let report = run(&service, plan, &[&view.docs.right]);
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(read(&real), b"Xone\ntwo\n");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("real.txt"));
    // The backup is the file's, next to it.
    assert_eq!(read(&w.join("real.txt.bak")), b"one\ntwo\n");
    assert!(!w.join("link.txt.bak").exists());
}

#[cfg(unix)]
#[test]
fn a10_17_a_fifo_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let r = write(&w, "r.txt", b"r\n");
    let fifo = w.join("fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        eprintln!("skipped: mkfifo is not available");
        return;
    }
    let mut view = open(&service, &l, &r);
    type_x(&mut view.docs.right);
    let req = SaveRequest {
        target: Some(spec(&service, &fifo)),
        ..in_place(&service)
    };
    let plan = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    let error = plan.plan.items[0].error.clone().expect("refused");
    assert!(
        error.message.contains("is not a regular file"),
        "{}",
        error.message
    );
    let report = run(&service, plan, &[&view.docs.right]);
    assert_eq!(report.status(0), Some(ItemStatus::Failed));
    assert_eq!(save_exit_code(&report), ExitCode::UnknownError);
    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo()
    );
}

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt as _;

// ---- Save As -----------------------------------------------------------------------------------------------------

#[test]
fn a10_17_save_as_moves_the_document() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let r = write(&w, "r.txt", b"one\ntwo\n");
    let mut view = open(&service, &l, &r);
    type_x(&mut view.docs.right);
    let new = w.join("new.txt");
    let req = SaveRequest {
        target: Some(spec(&service, &new)),
        ..in_place(&service)
    };
    let first = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    assert_eq!(first.items[0].mode, SaveMode::SaveAs);
    assert!(first.items[0].creates);
    assert!(first.items[0].questions().is_empty());
    let report = run(&service, first, &[&view.docs.right]);
    assert!(report.ok());
    apply_save_report(&report, &mut [&mut view.docs.right]);
    assert_eq!(read(&new), b"Xone\ntwo\n");
    assert_eq!(read(&r), b"one\ntwo\n");
    assert_eq!(
        view.docs.right.location().map(Spec::canonical),
        Some(spec(&service, &new).canonical())
    );
    // The next Save goes to the new file, splicing its unchanged line.
    type_x(&mut view.docs.right);
    let req = in_place(&service);
    let next = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    assert_eq!(next.items[0].mode, SaveMode::InPlace);
    assert_eq!(
        next.items[0].target.canonical(),
        spec(&service, &new).canonical()
    );
    assert!(next.items[0].content.spliced_bytes > 0);
    run(&service, next, &[&view.docs.right]);
    assert_eq!(read(&new), b"XXone\ntwo\n");
    assert_eq!(read(&r), b"one\ntwo\n");
}

// ---- --savetarget ------------------------------------------------------------------------------------------------

#[test]
fn a10_17_savetarget_redirects_the_right_pane_and_the_left_saves_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"left\n");
    let r = write(&w, "r.txt", b"right\nside\n");
    let target = write(&w, "target.txt", b"old target\n");
    let raw = save_target_spec(target.as_os_str(), &service.spec_context()).unwrap();
    let opts = LiveOptions {
        save_target: Some(raw),
        ..LiveOptions::default()
    };
    let mut view = open_with(&service, &l, &r, opts);
    assert_eq!(
        view.docs.right.save_target().map(Spec::canonical),
        Some(spec(&service, &target).canonical())
    );
    assert!(view.docs.left.save_target().is_none());
    type_x(&mut view.docs.left);
    type_x(&mut view.docs.right);
    let left_req = service.live_save_request(&view, Side::Left).unwrap();
    let right_req = service.live_save_request(&view, Side::Right).unwrap();
    let docs = [
        (Some(Side::Left), &view.docs.left, &left_req),
        (Some(Side::Right), &view.docs.right, &right_req),
    ];
    let first = plan(&service, &docs);
    assert_eq!(first.items[0].mode, SaveMode::InPlace);
    assert_eq!(first.items[1].mode, SaveMode::SaveTarget);
    // An existing target is asked about once.
    assert_eq!(
        first.items[1].questions(),
        vec![SaveQuestion::ReplaceExisting]
    );
    let report = run(&service, first, &[&view.docs.left, &view.docs.right]);
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(read(&l), b"Xleft\n");
    assert_eq!(read(&target), b"Xright\nside\n");
    assert_eq!(read(&r), b"right\nside\n");
    assert_eq!(read(&w.join("target.txt.bak")), b"old target\n");
    apply_save_report(&report, &mut [&mut view.docs.left, &mut view.docs.right]);
    // The location stays; the target is the next splice source, saved again without a question.
    assert_eq!(
        view.docs.right.location().map(Spec::canonical),
        Some(spec(&service, &r).canonical())
    );
    type_x(&mut view.docs.right);
    let right_req = service.live_save_request(&view, Side::Right).unwrap();
    let again = plan(
        &service,
        &[(Some(Side::Right), &view.docs.right, &right_req)],
    );
    assert!(
        again.items[0].questions().is_empty(),
        "{:?}",
        again.items[0].issues
    );
    assert!(again.items[0].content.spliced_bytes > 0);
    run(&service, again, &[&view.docs.right]);
    assert_eq!(read(&target), b"XXright\nside\n");
    assert_eq!(read(&r), b"right\nside\n");
    // `-` is not a save target.
    let opts = LiveOptions {
        save_target: Some("-".into()),
        ..LiveOptions::default()
    };
    let req = TextCompareRequest {
        left: os(&l),
        right: os(&r),
        ..TextCompareRequest::default()
    };
    assert!(
        service
            .text_compare_live(&req, opts, &job(&service, JobKind::TextCompare), None)
            .is_err()
    );
}

/// W10b note 59: the right pane of a version-control tool is a read-only temporary file whose `--savetarget` is the
/// writable working copy: the pane takes its writability from the target, so it is editable and saves there.
#[test]
fn a10_17_a_read_only_right_file_with_a_writable_savetarget_is_editable() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "base.txt", b"base\n");
    let r = write(&w, "tmp-right.txt", b"right\n");
    let target = write(&w, "working.txt", b"right\n");
    let mut perms = std::fs::metadata(&r).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&r, perms).unwrap();
    // Without a target the read-only file blocks the pane (unless the process may write anything, as root).
    let view = open(&service, &l, &r);
    let read_only = view.docs.right.editability().first();
    let raw = save_target_spec(target.as_os_str(), &service.spec_context()).unwrap();
    let opts = LiveOptions {
        save_target: Some(raw),
        ..LiveOptions::default()
    };
    let mut view = open_with(&service, &l, &r, opts);
    assert_eq!(
        view.docs.right.editability().first(),
        None,
        "was {read_only:?}"
    );
    hikasa_session::api::ensure_editable(Some(Side::Right), &view.docs.right.editability())
        .unwrap();
    type_x(&mut view.docs.right);
    let req = service.live_save_request(&view, Side::Right).unwrap();
    let first = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    let report = run(&service, first, &[&view.docs.right]);
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(read(&target), b"Xright\n");
    assert_eq!(read(&r), b"right\n");
    // A read-only target makes the pane read-only, whatever the right file is.
    let mut perms = std::fs::metadata(&r).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&r, perms).unwrap();
    let mut perms = std::fs::metadata(&target).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&target, perms).unwrap();
    if read_only.is_some() {
        let raw = save_target_spec(target.as_os_str(), &service.spec_context()).unwrap();
        let opts = LiveOptions {
            save_target: Some(raw),
            ..LiveOptions::default()
        };
        let view = open_with(&service, &l, &r, opts);
        assert!(view.docs.right.editability().first().is_some());
    }
}

#[test]
fn a10_17_savetarget_of_a_text_edit_document() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let file = write(&w, "edit.txt", b"text\n");
    let target = w.join("out.txt");
    // A Text Edit view opened with `--savetarget` (W10c's `text_edit_open`; the target made absolute as
    // `resolve_open` makes it).
    let raw = save_target_spec(target.as_os_str(), &service.spec_context()).unwrap();
    let mut resp = service
        .text_edit_open(
            &TextEditRequest {
                spec: file.as_os_str().to_owned(),
                save_target: Some(raw),
                ..TextEditRequest::default()
            },
            &job(&service, JobKind::TextCompare),
        )
        .unwrap();
    assert_eq!(
        resp.doc.save_target().map(Spec::canonical),
        Some(spec(&service, &target).canonical())
    );
    type_x(&mut resp.doc);
    let req = service.text_edit_save_request(&resp).unwrap();
    let plan = plan(&service, &[(None, &resp.doc, &req)]);
    assert_eq!(plan.items[0].side, None);
    assert_eq!(plan.items[0].mode, SaveMode::SaveTarget);
    assert!(plan.items[0].questions().is_empty());
    let report = run(&service, plan, &[&resp.doc]);
    assert!(report.ok());
    apply_save_report(&report, &mut [&mut resp.doc]);
    assert_eq!(read(&target), b"Xtext\n");
    assert_eq!(read(&file), b"text\n");
    assert_eq!(
        resp.doc.location().map(Spec::canonical),
        Some(spec(&service, &file).canonical())
    );
}

// ---- backups -----------------------------------------------------------------------------------------------------

#[test]
fn a10_17_every_naming_style_once_and_the_folder_style() {
    let styles = [
        ("name_tilde", "r.txt~"),
        ("stem_tilde", "r~.txt"),
        ("name_bak", "r.txt.bak"),
        ("tilde_name", "~r.txt"),
        ("stem_bak", "r.bak"),
        ("stem_bak_ext", "r.bak.txt"),
    ];
    for (naming, name) in styles {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "config/options.toml",
            format!("[backups]\nnaming = \"{naming}\"\n").as_bytes(),
        );
        let service = service(tmp.path());
        let w = tmp.path().join("work");
        let l = write(&w, "l.txt", b"l\n");
        let r = write(&w, "r.txt", b"r\n");
        let mut view = open(&service, &l, &r);
        type_x(&mut view.docs.right);
        let req = service.live_save_request(&view, Side::Right).unwrap();
        let report = run(
            &service,
            plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]),
            &[&view.docs.right],
        );
        assert!(report.ok(), "{naming}: {:?}", report.op.results);
        assert_eq!(read(&w.join(name)), b"r\n", "{naming}");
        assert_eq!(read(&r), b"Xr\n");
    }
    // `folder`: `<folder>/<file name>`, the folder made on demand.
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("backups").join("deep");
    let text = format!(
        "[backups]\nnaming = \"folder\"\nfolder = {}\n",
        toml_string(&folder)
    );
    write(tmp.path(), "config/options.toml", text.as_bytes());
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let r = write(&w, "r.txt", b"r\n");
    let mut view = open(&service, &l, &r);
    type_x(&mut view.docs.right);
    let req = service.live_save_request(&view, Side::Right).unwrap();
    let report = run(
        &service,
        plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]),
        &[&view.docs.right],
    );
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(read(&folder.join("r.txt")), b"r\n");
    // A relative folder is refused when the policy is made.
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "config/options.toml",
        b"[backups]\nnaming = \"folder\"\nfolder = \"relative/dir\"\n",
    );
    let refusing = common::service(tmp.path());
    assert!(refusing.backup_policy_for_save(false).is_err());
    assert!(refusing.backup_policy_for_save(true).unwrap().is_none());
}

/// A TOML basic string of a path.
fn toml_string(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('\\', "\\\\"))
}

#[test]
fn a10_17_nobackups_and_back_up_before_save() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let r = write(&w, "r.txt", b"r\n");
    // --nobackups: none.
    let opts = LiveOptions {
        no_backups: true,
        ..LiveOptions::default()
    };
    let mut view = open_with(&service, &l, &r, opts);
    type_x(&mut view.docs.right);
    let req = service.live_save_request(&view, Side::Right).unwrap();
    assert!(req.backup.is_none());
    run(
        &service,
        plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]),
        &[&view.docs.right],
    );
    assert_eq!(read(&r), b"Xr\n");
    assert!(!w.join("r.txt.bak").exists());
    // `back_up_before_save = false` in the settings: none either; `--backup` (`Some(true)`) turns them on.
    write(
        tmp.path(),
        "config/options.toml",
        b"[backups]\nback_up_before_save = false\n",
    );
    assert!(service.backup_policy_for_save(false).unwrap().is_none());
    assert!(
        service
            .backup_policy_for_save_with(Some(true))
            .unwrap()
            .is_some()
    );
    // Without either: the default is on.
    std::fs::remove_file(tmp.path().join("config/options.toml")).unwrap();
    let mut view = open(&service, &l, &r);
    type_x(&mut view.docs.right);
    let req = service.live_save_request(&view, Side::Right).unwrap();
    assert!(req.backup.is_some());
    run(
        &service,
        plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]),
        &[&view.docs.right],
    );
    assert_eq!(read(&w.join("r.txt.bak")), b"Xr\n");
}

#[test]
fn a10_17_a_backup_that_fails_fails_the_item() {
    let tmp = tempfile::tempdir().unwrap();
    // The backup folder cannot be made: a file stands where its parent should be.
    let blocker = write(tmp.path(), "blocker", b"a file\n");
    let folder = blocker.join("sub");
    let text = format!(
        "[backups]\nnaming = \"folder\"\nfolder = {}\n",
        toml_string(&folder)
    );
    write(tmp.path(), "config/options.toml", text.as_bytes());
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let r = write(&w, "r.txt", b"r\n");
    let mut view = open(&service, &l, &r);
    type_x(&mut view.docs.right);
    let req = service.live_save_request(&view, Side::Right).unwrap();
    let plan = plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]);
    let report = run(&service, plan, &[&view.docs.right]);
    assert_eq!(
        report.status(0),
        Some(ItemStatus::Failed),
        "{:?}",
        report.op.results
    );
    assert_eq!(save_exit_code(&report), ExitCode::UnknownError);
    assert_eq!(read(&r), b"r\n");
    assert_eq!(apply_save_report(&report, &mut [&mut view.docs.right]), 0);
    assert!(view.docs.right.is_modified());
}

// ---- A10.32 ------------------------------------------------------------------------------------------------------

/// A file whose name is not valid UTF-8 is saved in place with its backup (Linux).
#[cfg(target_os = "linux")]
#[test]
fn a10_32_a_non_utf8_file_name_is_saved() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l.txt", b"l\n");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    let r = w.join(name);
    std::fs::write(&r, b"r\n").unwrap();
    let mut view = open(&service, &l, &r);
    assert!(view.docs.right.editability().editable());
    type_x(&mut view.docs.right);
    let req = in_place(&service);
    let report = run(
        &service,
        plan(&service, &[(Some(Side::Right), &view.docs.right, &req)]),
        &[&view.docs.right],
    );
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(read(&r), b"Xr\n");
    let mut backup = name.to_owned();
    backup.push(".bak");
    assert_eq!(read(&w.join(backup)), b"r\n");
}

/// Windows targets of Save As (the spec the window parses) and `--savetarget` (the canonical string the CLI sends and
/// the Service parses again), as pure tables on every platform.
#[test]
fn a10_32_windows_targets_of_save_as_and_savetarget() {
    let windows = SpecContext {
        style: PathStyle::Windows,
        cwd: Some(PathBuf::from("C:\\work")),
        home: Some(PathBuf::from("C:\\Users\\me")),
    };
    let rows: &[(&str, &str)] = &[
        ("out.txt", "C:\\work\\out.txt"),
        ("c:\\Work\\..\\out.txt", "C:\\out.txt"),
        ("D:/docs/new.txt", "D:\\docs\\new.txt"),
        (
            "\\\\server\\share\\dir\\x.txt",
            "\\\\server\\share\\dir\\x.txt",
        ),
        ("//server/share/x.txt", "\\\\server\\share\\x.txt"),
        (
            "\\\\?\\C:\\very\\long\\x.txt",
            "\\\\?\\C:\\very\\long\\x.txt",
        ),
        (
            "\\\\?\\UNC\\server\\share\\x.txt",
            "\\\\?\\UNC\\server\\share\\x.txt",
        ),
        ("D:\\a.zip\\inner\\x.txt", "D:\\a.zip\\inner\\x.txt"),
    ];
    for (raw, want) in rows {
        let raw = OsString::from(raw);
        // Save As: the target spec.
        let save_as = Spec::parse_os(&raw, &windows).unwrap();
        assert_eq!(save_as.canonical(), *want, "{raw:?}");
        // --savetarget: the CLI's canonical string, then the Service's parse of it, the same spec.
        let sent = save_target_spec(&raw, &windows).unwrap();
        assert_eq!(sent, *want, "{raw:?}");
        let back = Spec::parse_os(OsString::from(&sent).as_os_str(), &windows).unwrap();
        assert_eq!(back, save_as, "{raw:?}");
    }
}
