//! M2 W9c acceptance A9.22 (core, D-254): *Show in File Manager*. The command line `SystemLauncher::reveal` runs on
//! each platform (the pure `reveal_command`, never run here), the `file://` URI of the D-Bus call, the folder of the
//! fallback, `RecordingLauncher`'s record, and the Service's targets: a local item, a member (its outermost archive),
//! a spec that is not local (refused).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{facts, fixed_clock, job};
use hikasa_core::{JobKind, LocalPath, Spec};
use hikasa_session::api::file_manager_targets;
use hikasa_session::host::{Platform, file_uri, reveal_command, reveal_fallback};
use hikasa_session::launcher::{AppLauncher, NoLauncher, Recorded, RecordingLauncher};
use hikasa_session::{ApiError, Service, ServiceInit};

fn texts(args: &[std::ffi::OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a9_22_the_command_line_of_each_platform() {
    let windows = reveal_command(Platform::Windows, Path::new(r"C:\Data\a b.txt"));
    assert_eq!(windows.program, "explorer.exe");
    assert_eq!(texts(&windows.args), [r#"/select,"C:\Data\a b.txt""#]);
    let mac = reveal_command(Platform::MacOs, Path::new("/Users/me/a b.txt"));
    assert_eq!(mac.program, "open");
    assert_eq!(texts(&mac.args), ["-R", "/Users/me/a b.txt"]);
    assert_eq!(mac.cwd, PathBuf::from("/Users/me"));
    for platform in [Platform::Linux, Platform::OtherUnix] {
        let linux = reveal_command(platform, Path::new("/home/me/a b,c%.txt"));
        assert_eq!(linux.program, "dbus-send");
        assert_eq!(
            texts(&linux.args),
            [
                "--session",
                "--print-reply",
                "--reply-timeout=5000",
                "--type=method_call",
                "--dest=org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                "array:string:file:///home/me/a%20b%2Cc%25.txt",
                "string:",
            ]
        );
        assert_eq!(linux.cwd, PathBuf::from("/home/me"));
    }
}

#[test]
fn a9_22_uris_and_the_fallback_folder() {
    assert_eq!(file_uri(Path::new("/a/b")), "file:///a/b");
    assert_eq!(
        file_uri(Path::new("/x/é ü~_-.")),
        "file:///x/%C3%A9%20%C3%BC~_-."
    );
    assert_eq!(file_uri(Path::new("/q/a?#[]")), "file:///q/a%3F%23%5B%5D");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/d/caf\xe9"));
        assert_eq!(file_uri(odd), "file:///d/caf%E9");
    }
    assert_eq!(
        reveal_fallback(Path::new("/a/b/c.txt")),
        PathBuf::from("/a/b")
    );
    assert_eq!(reveal_fallback(Path::new("/")), PathBuf::from("/"));
}

#[test]
fn a9_22_the_recording_launcher_records_and_the_default_refuses() {
    let launcher = RecordingLauncher::with_answers(vec![Some(0), None]);
    launcher.reveal(Path::new("/a/one")).unwrap();
    let err = launcher.reveal(Path::new("/a/two")).unwrap_err();
    assert!(err.to_string().contains("/a/two"), "{err}");
    assert_eq!(
        launcher.calls(),
        [
            Recorded::Reveal(PathBuf::from("/a/one")),
            Recorded::Reveal(PathBuf::from("/a/two"))
        ]
    );
    assert!(matches!(
        NoLauncher.reveal(Path::new("/a")),
        Err(ApiError::Unsupported(_))
    ));
}

fn service(root: &Path, launcher: Arc<RecordingLauncher>) -> Service {
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

fn local(path: &Path) -> Spec {
    Spec::Local(LocalPath::new(path.to_path_buf()).unwrap())
}

#[test]
fn a9_22_the_service_reveals_local_items_and_members_by_their_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let launcher = Arc::new(RecordingLauncher::default());
    let svc = service(&root, Arc::clone(&launcher));
    let control = job(&svc, JobKind::VfsRead);
    let file = root.join("data").join("a.txt");
    let archive = root.join("data").join("outer.zip");
    let member = Spec::archive_member(
        Spec::archive_member(local(&archive), "inner.tar"),
        "deep/b.txt",
    );
    svc.reveal_in_file_manager(&local(&file), &control).unwrap();
    svc.reveal_in_file_manager(&member, &control).unwrap();
    svc.open_folder_in_file_manager(&local(&file), false, &control)
        .unwrap();
    svc.open_folder_in_file_manager(&local(&root.join("data")), true, &control)
        .unwrap();
    svc.open_folder_in_file_manager(&member, false, &control)
        .unwrap();
    assert_eq!(
        launcher.calls(),
        [
            Recorded::Reveal(file.clone()),
            Recorded::Reveal(archive.clone()),
            Recorded::Associated(root.join("data")),
            Recorded::Associated(root.join("data")),
            Recorded::Associated(root.join("data")),
        ]
    );
    // Not on a local disk: refused, nothing started; the pure targets say the same.
    for spec in [Spec::Clipboard, Spec::Stdin] {
        assert!(file_manager_targets(&spec, false).is_none());
        let err = svc.reveal_in_file_manager(&spec, &control).unwrap_err();
        assert!(matches!(err, ApiError::Unsupported(_)), "{err:?}");
        let err = svc
            .open_folder_in_file_manager(&spec, true, &control)
            .unwrap_err();
        assert!(matches!(err, ApiError::Unsupported(_)), "{err:?}");
    }
    assert_eq!(launcher.calls().len(), 5);
    let targets = file_manager_targets(&member, true).unwrap();
    assert_eq!(targets.item, archive);
    assert_eq!(
        targets.folder,
        root.join("data"),
        "a member is never a folder of the disk"
    );
}
