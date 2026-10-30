//! M2 W3 acceptance A3.8, the recycle-bin parts (D-98): `MemoryVfs::trashed()` order, one `trash` item per root,
//! a trash fault leaves the item in place and never removes it permanently, blobs report `OperationUnsupported`,
//! and the real recycle bin of `LocalVfs` on Linux.
//!
//! The real-trash test never touches the user's recycle bin: the parent test re-runs this test binary for the
//! single ignored child test `real_trash_child`, with `HOME` and `XDG_DATA_HOME` pointing into a tempdir that
//! also holds the file (so the home trash on that file system is used), and the variables set **only** for that
//! child process. The child refuses to call the trash unless both variables point into its own tempdir. On
//! other platforms the real-trash test is `#[ignore]` (manual).

mod common;

use std::sync::Arc;

use common::Fx;
use hikasa_core::{CaseSensitivity, Ctl, ErrorKind, FileTime, Spec};
use hikasa_vfs::ops::{self, Action, ItemStatus, OpReport, RemoveOptions};
use hikasa_vfs::{Blob, EntryKind, Fault, LocalVfs, Vfs, VfsError, VfsOp, VfsRouter};

const TRASH: RemoveOptions = RemoveOptions {
    recursive: false,
    trash: true,
};

const TRASH_RECURSIVE: RemoveOptions = RemoveOptions {
    recursive: true,
    trash: true,
};

fn trash(vfs: &dyn Vfs, specs: &[Spec], opts: &RemoveOptions) -> OpReport {
    let plan = ops::plan_remove(vfs, specs, opts, &Ctl::detached()).unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn statuses(report: &OpReport) -> Vec<(Action, ItemStatus)> {
    report
        .plan
        .items
        .iter()
        .zip(&report.results)
        .map(|(i, r)| (i.action, r.status))
        .collect()
}

#[test]
fn a3_8_memory_trashed_order_and_one_item_per_root() {
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let fx = Fx::mem(case);
        fx.file("b", b"b");
        fx.file("a", b"a");
        fx.file("dir/x", b"x");
        fx.file("dir/sub/y", b"y");
        fx.dir("empty");
        fx.link("l", "a", false);
        // A non-empty folder still needs `recursive`.
        let report = trash(fx.vfs(), &[fx.spec("dir")], &TRASH);
        assert_eq!(statuses(&report), [(Action::Trash, ItemStatus::Failed)]);
        assert_eq!(
            report.results[0].error.as_ref().unwrap().kind,
            ErrorKind::NotEmpty
        );
        assert!(fx.memory().trashed().is_empty());
        // Argument order; one item per root, the folder as a whole.
        let specs = ["b", "l", "dir", "a", "empty"].map(|r| fx.spec(r));
        let report = trash(fx.vfs(), &specs, &TRASH_RECURSIVE);
        assert_eq!(statuses(&report), [(Action::Trash, ItemStatus::Done); 5]);
        assert_eq!(
            fx.memory().trashed(),
            ["b", "l", "dir", "a", "empty"].map(|r| fx.text(r)).to_vec()
        );
        assert!(fx.names("").is_empty(), "{:?}", fx.names(""));
    }
}

#[test]
fn a3_8_trash_fault_leaves_the_item_and_never_removes_it() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("a", b"a");
    fx.file("dir/x", b"x");
    fx.file("c", b"c");
    for path in ["a", "dir"] {
        fx.memory().inject(Fault {
            path: fx.text(path),
            op: VfsOp::Trash,
            kind: ErrorKind::PermissionDenied,
        });
    }
    fx.memory().set_journal(true);
    let before = fx.memory().dump();
    let report = trash(
        fx.vfs(),
        &[fx.spec("a"), fx.spec("dir"), fx.spec("c")],
        &TRASH_RECURSIVE,
    );
    assert_eq!(
        statuses(&report),
        [
            (Action::Trash, ItemStatus::Failed),
            (Action::Trash, ItemStatus::Failed),
            (Action::Trash, ItemStatus::Done),
        ]
    );
    for result in &report.results[..2] {
        assert_eq!(
            result.error.as_ref().unwrap().kind,
            ErrorKind::PermissionDenied
        );
    }
    assert!(!report.ok());
    assert_eq!(fx.memory().trashed(), [fx.text("c")]);
    // Never a permanent delete: no `remove` call at all, and the failed items are exactly as before.
    assert_eq!(
        fx.memory().journal(),
        [(VfsOp::Trash, fx.spec("c").canonical())]
    );
    let after = fx.memory().dump();
    let kept: Vec<_> = before
        .iter()
        .filter(|d| !d.path.ends_with('c'))
        .cloned()
        .collect();
    assert_eq!(after, kept);
    assert_eq!(fx.read("dir/x"), b"x");
}

fn blob() -> Blob {
    Blob {
        bytes: Arc::from(&b"blob"[..]),
        mtime: FileTime::from_unix(1_700_000_000, 0).unwrap(),
        name: Some("blob.txt".into()),
    }
}

#[test]
fn a3_8_blobs_report_operation_unsupported() {
    let router = VfsRouter::new(LocalVfs::default())
        .with_stdin(blob())
        .with_clipboard(blob());
    for spec in [Spec::Stdin, Spec::Clipboard] {
        let err = router.move_to_trash(&spec).unwrap_err();
        assert!(
            matches!(
                err,
                VfsError::OperationUnsupported {
                    op: VfsOp::Trash,
                    ..
                }
            ),
            "{spec:?}: {err:?}"
        );
        assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::Unsupported);
        // Through ops: the item fails `unsupported`.
        for opts in [TRASH, RemoveOptions::default()] {
            let report = trash(&router, std::slice::from_ref(&spec), &opts);
            assert_eq!(report.results.len(), 1);
            assert_eq!(report.results[0].status, ItemStatus::Failed, "{spec:?}");
            assert_eq!(
                report.results[0].error.as_ref().unwrap().kind,
                ErrorKind::Unsupported,
                "{spec:?} {opts:?}: {:?}",
                report.results[0].error
            );
        }
    }
    // Without a blob: still unsupported, never a local call.
    let bare = VfsRouter::new(LocalVfs::default());
    assert!(matches!(
        bare.move_to_trash(&Spec::Clipboard),
        Err(VfsError::OperationUnsupported {
            op: VfsOp::Trash,
            ..
        })
    ));
}

/// The environment variable that turns `real_trash_child` on (set only for the child process).
const CHILD_DIR: &str = "HIKASA_TEST_TRASH_CHILD_DIR";

/// The child half of the real-trash test: does nothing unless the parent set `CHILD_DIR`, and refuses to call
/// the recycle bin unless `HOME` and `XDG_DATA_HOME` both point into that folder.
#[test]
#[ignore = "child process of a3_8_local_real_trash_in_an_isolated_home"]
fn real_trash_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for var in ["HOME", "XDG_DATA_HOME"] {
        let value = std::path::PathBuf::from(std::env::var_os(var).expect(var));
        assert!(
            value.starts_with(&dir),
            "{var}={} is not inside {}: refusing to touch a real recycle bin",
            value.display(),
            dir.display()
        );
    }
    let local = LocalVfs::default();
    let file = common::lspec(&dir.join("victim.txt"));
    let mut specs = vec![file];
    if dir.join("alink").symlink_metadata().is_ok() {
        specs.push(common::lspec_new(&dir.join("alink")));
    }
    let report = trash(&local, &specs, &TRASH);
    assert!(
        report.results.iter().all(|r| r.status == ItemStatus::Done),
        "{:?}",
        report.results
    );
    println!("child: trashed");
}

#[cfg(target_os = "linux")]
#[test]
fn a3_8_local_real_trash_in_an_isolated_home() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let home = dir.join("home");
    let data = home.join(".local").join("share");
    std::fs::create_dir_all(&data).unwrap();
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, b"to the bin").unwrap();
    // A link (A3.6: trash moves the link, never what it points to).
    let pointed = dir.join("pointed.txt");
    std::fs::write(&pointed, b"stays").unwrap();
    std::os::unix::fs::symlink("pointed.txt", dir.join("alink")).unwrap();

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "real_trash_child",
            "--nocapture",
            "--include-ignored",
        ])
        .env(CHILD_DIR, &dir)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "child failed\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("child: trashed"),
        "the child did not run\n{stdout}\n{stderr}"
    );

    assert!(!victim.exists(), "the file left its place");
    let files = data.join("Trash").join("files");
    let info = data.join("Trash").join("info");
    let trashed: Vec<_> = std::fs::read_dir(&files)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let mut trashed = trashed;
    trashed.sort();
    assert_eq!(trashed, ["alink", "victim.txt"]);
    // The link went to the bin as a link with its target text; the file it points to is untouched.
    assert!(!dir.join("alink").exists() && dir.join("alink").symlink_metadata().is_err());
    let in_bin = files.join("alink");
    assert!(in_bin.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&in_bin).unwrap(),
        std::path::Path::new("pointed.txt")
    );
    assert_eq!(std::fs::read(&pointed).unwrap(), b"stays");
    assert!(info.join("alink.trashinfo").exists());
    assert_eq!(
        std::fs::read(files.join("victim.txt")).unwrap(),
        b"to the bin"
    );
    let text = std::fs::read_to_string(info.join("victim.txt.trashinfo")).unwrap();
    assert!(text.contains("[Trash Info]"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("Path=") && l.contains("victim.txt")),
        "{text}"
    );
    assert!(
        text.lines().any(|l| l.starts_with("DeletionDate=")),
        "{text}"
    );
    assert!(
        LocalVfs::default()
            .stat(
                &common::lspec(&files.join("victim.txt")),
                hikasa_vfs::FollowLinks::No
            )
            .is_ok_and(|e| e.kind == EntryKind::File)
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
#[ignore = "manual: moves a file to the real recycle bin of this platform"]
fn a3_8_local_real_trash_in_an_isolated_home() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, b"to the bin").unwrap();
    let report = trash(&LocalVfs::default(), &[common::lspec(&victim)], &TRASH);
    assert!(report.ok(), "{:?}", report.results);
    assert!(!victim.exists());
}
