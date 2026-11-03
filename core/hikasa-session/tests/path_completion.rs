//! M2 W9a acceptance A9.5 (core, D-239) and the create-missing calls (D-251's `vfs_op_plan`/`vfs_op_execute`):
//! `Service::path_completions` on disk, `~`, a zip and a member folder; the Windows drive and UNC forms of the typed
//! text (pure table); the prefix by the back-end's case rule; folders and archives first; `limit` and `truncated`; a
//! missing folder; a cancel. The browse places with an injected drive probe.

mod common;

use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::JobKind;
use hikasa_core::spec::PathStyle;
use hikasa_session::api::paths::{PlaceKind, completion_matches, places, split_typed};
use hikasa_session::api::vfs_ops::VfsOpRequest;
use hikasa_session::host::Platform;

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn the_typed_text_splits_per_style() {
    let unix = [
        ("/a/b", ("/a/", "b")),
        ("/a/", ("/a/", "")),
        ("~/Do", ("~/", "Do")),
        ("abc", ("", "abc")),
        ("/", ("/", "")),
        ("a\\b", ("", "a\\b")),
        ("x.zip/in/m", ("x.zip/in/", "m")),
    ];
    for (typed, expected) in unix {
        assert_eq!(split_typed(typed, PathStyle::Unix), expected, "{typed}");
    }
    let windows = [
        ("C:\\a\\b", ("C:\\a\\", "b")),
        ("C:/a/b", ("C:/a/", "b")),
        ("C:x", ("C:", "x")),
        ("C:", ("C:", "")),
        ("c:\\", ("c:\\", "")),
        ("\\\\srv\\share\\d", ("\\\\srv\\share\\", "d")),
        ("\\\\?\\C:\\x\\y", ("\\\\?\\C:\\x\\", "y")),
        ("name", ("", "name")),
    ];
    for (typed, expected) in windows {
        assert_eq!(split_typed(typed, PathStyle::Windows), expected, "{typed}");
    }
}

#[test]
fn the_prefix_follows_the_back_ends_case_rule() {
    assert!(completion_matches("Alpha", "al", false));
    assert!(!completion_matches("Alpha", "al", true));
    assert!(completion_matches("alpha", "al", true));
    assert!(completion_matches("anything", "", true));
    // NFC and NFD of "é" are one name for the prefix.
    assert!(completion_matches("\u{e9}t\u{e9}", "e\u{301}", true));
    assert!(completion_matches("\u{c9}T\u{c9}", "e\u{301}t", false));
    assert!(!completion_matches("ab", "abc", false));
}

/// True when the folder's back-end compares names case-sensitively (ext4: yes; APFS and NTFS: no).
fn case_sensitive(dir: &Path) -> bool {
    let svc = service(dir);
    let stat = svc
        .vfs_stat(dir.as_os_str(), hikasa_vfs::FollowLinks::Yes)
        .unwrap();
    stat.capabilities.contains(&"case_sensitive")
}

#[test]
fn completions_on_disk_home_and_archives() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let svc = service(&root);
    let dir = root.join("d");
    std::fs::create_dir_all(dir.join("Alpha")).unwrap();
    write(&dir, "alpine.txt", b"a");
    write(&dir, "beta", b"b");
    write(&dir, ".hidden", b"h");
    std::fs::write(
        dir.join("ALso.zip"),
        zip_bytes(&[("inner/x.txt", b"x"), ("top.txt", b"t")]),
    )
    .unwrap();
    let sep = std::path::MAIN_SEPARATOR;
    let base = format!("{}{sep}", dir.display());
    let all = svc
        .path_completions(OsStr::new(&base), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    assert_eq!(all.base, base);
    assert!(!all.truncated);
    let names: Vec<&str> = all.items.iter().map(|c| c.name.as_str()).collect();
    // Folders and archives first, then by the case-folded name.
    assert_eq!(
        names,
        ["Alpha", "ALso.zip", ".hidden", "alpine.txt", "beta"]
    );
    let alpha = &all.items[0];
    assert!(alpha.folder && !alpha.archive);
    assert_eq!(alpha.text, format!("{base}Alpha{sep}"));
    assert!(all.items[1].archive && !all.items[1].folder);
    assert_eq!(all.items[1].text, format!("{base}ALso.zip"));
    // The prefix by the disk's case rule.
    let typed = format!("{base}al");
    let some = svc
        .path_completions(OsStr::new(&typed), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    let names: Vec<&str> = some.items.iter().map(|c| c.name.as_str()).collect();
    if case_sensitive(&dir) {
        assert_eq!(names, ["alpine.txt"]);
    } else {
        assert_eq!(names, ["Alpha", "ALso.zip", "alpine.txt"]);
    }
    // `limit` and `truncated`.
    let two = svc
        .path_completions(OsStr::new(&base), 2, &job(&svc, JobKind::VfsList))
        .unwrap();
    assert_eq!(two.items.len(), 2);
    assert!(two.truncated);
    // A zip and a member folder (the archive's members, D-137).
    let zip = format!("{base}ALso.zip/");
    let members = svc
        .path_completions(OsStr::new(&zip), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    let names: Vec<&str> = members.items.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["inner", "top.txt"]);
    assert_eq!(members.items[0].text, format!("{zip}inner{sep}"));
    let inner = format!("{zip}inner/x");
    let deep = svc
        .path_completions(OsStr::new(&inner), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    assert_eq!(deep.items.len(), 1);
    assert_eq!(deep.items[0].text, format!("{zip}inner/x.txt"));
    // `~` is the home folder.
    std::fs::create_dir_all(root.join("home").join("docs")).unwrap();
    write(&root.join("home"), "dull", b"d");
    let home = svc
        .path_completions(OsStr::new("~/d"), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    let texts: Vec<&str> = home.items.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, [format!("~/docs{sep}").as_str(), "~/dull"]);
    // Relative to the current folder (`root/work`).
    write(&root.join("work"), "here.txt", b"h");
    let here = svc
        .path_completions(OsStr::new("he"), 200, &job(&svc, JobKind::VfsList))
        .unwrap();
    assert_eq!(here.base, "");
    assert_eq!(here.items[0].text, "here.txt");
}

#[test]
fn a_missing_folder_and_a_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let svc = service(&root);
    let missing = format!("{}/nope/x", root.display());
    let err = svc
        .path_completions(OsStr::new(&missing), 200, &job(&svc, JobKind::VfsList))
        .unwrap_err();
    assert_eq!(err.kind(), hikasa_core::ErrorKind::NotFound, "{err}");
    let control = job(&svc, JobKind::VfsList);
    control.cancel.cancel();
    let base = format!("{}/", root.display());
    assert!(matches!(
        svc.path_completions(OsStr::new(&base), 200, &control),
        Err(hikasa_session::ApiError::Cancelled)
    ));
}

#[test]
fn places_with_a_probe_table() {
    let home = Path::new("/h");
    let cwd = Path::new("/w");
    let recent = vec!["/r1".to_owned(), "/r2".to_owned()];
    let unix = places(Platform::Linux, Some(home), Some(cwd), &|_| true, &recent);
    let kinds: Vec<PlaceKind> = unix.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            PlaceKind::Home,
            PlaceKind::Current,
            PlaceKind::Root,
            PlaceKind::Recent,
            PlaceKind::Recent
        ]
    );
    assert_eq!(unix[2].spec, "/");
    let drives = |d: &str| d == "C:\\" || d == "Z:\\";
    let windows = places(Platform::Windows, None, None, &drives, &[]);
    let specs: Vec<(&str, &str)> = windows
        .iter()
        .map(|p| (p.label.as_str(), p.spec.as_str()))
        .collect();
    assert_eq!(specs, [("C:", "C:\\"), ("Z:", "Z:\\")]);
    assert!(windows.iter().all(|p| p.kind == PlaceKind::Drive));
}

#[test]
fn the_planned_mkdir_is_the_one_executed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let svc = service(&root);
    let target = root.join("new").join("deeper");
    let request = VfsOpRequest::Mkdir {
        specs: vec![os(&target)],
        parents: true,
    };
    let planned = svc
        .vfs_op_plan(&request, &job(&svc, JobKind::VfsWrite))
        .unwrap();
    assert!(planned.preview.dry_run);
    assert!(!target.exists(), "planning writes nothing");
    let asked = serde_json::to_string(&planned.plan).unwrap();
    let done = svc
        .vfs_op_execute(planned.plan.clone(), &job(&svc, JobKind::VfsWrite))
        .unwrap();
    assert!(target.is_dir());
    assert!(done.ok());
    // The executed plan is the plan asked about; the outcome equals `vfs_run` of the same request elsewhere.
    assert_eq!(serde_json::to_string(&planned.plan).unwrap(), asked);
    let other = root.join("other").join("deeper");
    let direct = svc
        .vfs_run(
            &VfsOpRequest::Mkdir {
                specs: vec![os(&other)],
                parents: true,
            },
            &job(&svc, JobKind::VfsWrite),
        )
        .unwrap();
    assert_eq!(direct.items.len(), done.items.len());
    assert_eq!(direct.summary, done.summary);
    for (a, b) in direct.items.iter().zip(&done.items) {
        assert_eq!((a.action, a.status), (b.action, b.status));
    }
}
