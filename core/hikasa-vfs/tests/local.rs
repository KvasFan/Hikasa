//! `LocalVfs` integration (§6.2.2): files, folders, symlinks, a broken symlink (`EntryError`), a non-UTF-8 name
//! (raw round trip), a permission-denied folder (skipped as root), the prefilter without `stat` (D-45), atomic
//! writes (INV-VFS-3), capabilities, `local_path` and `child`.

mod common;

use std::fs;
use std::io::Write;
use std::path::Path;

use common::{lspec, lspec_new};
use hikasa_core::{CancelToken, CaseSensitivity, Classify, ErrorKind, Spec};
use hikasa_vfs::{
    Capabilities, EntryKind, FileName, FollowLinks, ListOptions, LocalOptions, LocalVfs, Vfs,
    VfsError, WriteOptions,
};

fn list(vfs: &LocalVfs, dir: &Path, opts: &ListOptions<'_>) -> hikasa_vfs::Listing {
    let mut listing = vfs.list(&lspec(dir), opts, &CancelToken::never()).unwrap();
    listing.sort_by_name();
    listing
}

fn names(listing: &hikasa_vfs::Listing) -> Vec<&str> {
    listing.entries.iter().map(|e| e.name.as_str()).collect()
}

/// Leftover temporary files of atomic writes in `dir`.
fn temp_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("hikasa-tmp"))
        .collect()
}

#[test]
fn files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub").join("inner.bin"), [0u8; 3]).unwrap();
    let mtime = filetime::FileTime::from_unix_time(1_700_000_000, 123_456_700);
    filetime::set_file_mtime(dir.path().join("a.txt"), mtime).unwrap();
    let vfs = LocalVfs::default();

    let listing = list(&vfs, dir.path(), &ListOptions::default());
    assert_eq!(names(&listing), ["a.txt", "sub"]);
    assert!(listing.errors.is_empty() && listing.name_only.is_empty());
    let a = &listing.entries[0];
    assert_eq!((a.kind, a.size), (EntryKind::File, 5));
    assert_eq!(
        a.mtime,
        hikasa_core::FileTime::from_unix(1_700_000_000, 123_456_700)
    );
    assert!(!a.followed && a.link_target.is_none() && a.crc32.is_none());
    let sub = &listing.entries[1];
    assert_eq!((sub.kind, sub.size), (EntryKind::Dir, 0));
    assert_eq!(
        vfs.stat(&lspec(&dir.path().join("a.txt")), FollowLinks::No)
            .unwrap(),
        *a
    );
    assert_eq!(
        common::read_to_vec(vfs.open_read(&lspec(&dir.path().join("a.txt"))).unwrap()),
        b"hello"
    );
    assert!(matches!(
        vfs.open_read(&lspec(dir.path())),
        Err(VfsError::IsADirectory { .. })
    ));
    let missing = lspec_new(&dir.path().join("missing"));
    assert!(matches!(
        vfs.open_read(&missing),
        Err(VfsError::NotFound { .. })
    ));
    assert!(matches!(
        vfs.stat(&missing, FollowLinks::No),
        Err(VfsError::NotFound { .. })
    ));
    assert!(matches!(
        vfs.list(&missing, &ListOptions::default(), &CancelToken::never()),
        Err(VfsError::NotFound { .. })
    ));
    let file_as_dir = vfs
        .list(
            &lspec(&dir.path().join("a.txt")),
            &ListOptions::default(),
            &CancelToken::never(),
        )
        .unwrap_err();
    assert_eq!(file_as_dir.kind(), ErrorKind::NotADirectory);
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        vfs.list(&lspec(dir.path()), &ListOptions::default(), &token),
        Err(VfsError::Cancelled)
    ));
}

#[test]
fn large_folders_list_completely() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..2500 {
        fs::write(dir.path().join(format!("f{i:04}")), b"").unwrap();
    }
    let listing = list(&LocalVfs::default(), dir.path(), &ListOptions::default());
    assert_eq!(listing.entries.len(), 2500);
    assert_eq!(listing.entries[2499].name.as_str(), "f2499");
}

#[test]
fn capabilities_local_path_and_child() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), b"x").unwrap();
    let base = lspec(dir.path());
    let vfs = LocalVfs::default();
    let caps = vfs.capabilities(&base).unwrap();
    assert!(caps.contains(
        Capabilities::READ
            | Capabilities::WRITE
            | Capabilities::LIST
            | Capabilities::SEEK
            | Capabilities::MTIME
            | Capabilities::SYMLINKS
            | Capabilities::LOCAL_PATH
    ));
    assert_eq!(
        caps.contains(Capabilities::CASE_SENSITIVE),
        CaseSensitivity::HOST_DEFAULT == CaseSensitivity::Sensitive
    );
    if cfg!(unix) {
        assert!(caps.contains(Capabilities::UNIX_MODE | Capabilities::UNIX_OWNER));
        assert!(!caps.contains(Capabilities::DOS_ATTRS));
    }
    if cfg!(windows) {
        assert!(caps.contains(Capabilities::DOS_ATTRS));
    }
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let vfs = LocalVfs::new(LocalOptions { case: Some(case) });
        assert_eq!(
            vfs.capabilities(&base)
                .unwrap()
                .contains(Capabilities::CASE_SENSITIVE),
            case == CaseSensitivity::Sensitive
        );
    }
    assert!(matches!(
        vfs.capabilities(&Spec::Stdin),
        Err(VfsError::Unsupported {
            what: "non-local spec",
            ..
        })
    ));
    assert!(matches!(
        vfs.stat(&Spec::Clipboard, FollowLinks::No),
        Err(VfsError::Unsupported { .. })
    ));

    let child = vfs.child(&base, &FileName::new("f").unwrap()).unwrap();
    assert_eq!(child, lspec(&dir.path().join("f")));
    assert_eq!(
        vfs.local_path(&child),
        Some(dir.path().canonicalize().unwrap().join("f"))
    );
    assert_eq!(vfs.local_path(&base), None, "folders are not mapped");
    assert_eq!(vfs.local_path(&lspec_new(&dir.path().join("none"))), None);
    let bad = FileName::from_os(std::ffi::OsStr::new("a/b"));
    assert!(matches!(
        vfs.child(&base, &bad),
        Err(VfsError::InvalidName { .. })
    ));
}

/// INV-VFS-3 on the local file system.
#[test]
fn atomic_writes() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalVfs::default();
    let target = dir.path().join("report.txt");
    let spec = lspec_new(&target);

    let mut w = vfs.open_write(&spec, &WriteOptions::default()).unwrap();
    w.write_all(b"draft").unwrap();
    assert!(!target.exists(), "invisible before commit");
    assert_eq!(temp_files(dir.path()).len(), 1);
    w.commit().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"draft");
    assert!(temp_files(dir.path()).is_empty());

    // Drop discards.
    let dropped = dir.path().join("dropped.txt");
    let mut w = vfs
        .open_write(&lspec_new(&dropped), &WriteOptions::default())
        .unwrap();
    w.write_all(b"x").unwrap();
    drop(w);
    assert!(!dropped.exists());
    assert!(temp_files(dir.path()).is_empty());

    // overwrite = false.
    assert!(matches!(
        vfs.open_write(&spec, &WriteOptions::default()),
        Err(VfsError::AlreadyExists { .. })
    ));
    // A target created between open and commit is not replaced.
    let late = dir.path().join("late.txt");
    let mut w = vfs
        .open_write(&lspec_new(&late), &WriteOptions::default())
        .unwrap();
    w.write_all(b"mine").unwrap();
    fs::write(&late, b"theirs").unwrap();
    assert!(matches!(w.commit(), Err(VfsError::AlreadyExists { .. })));
    assert_eq!(fs::read(&late).unwrap(), b"theirs");
    assert!(temp_files(dir.path()).is_empty());

    // overwrite = true replaces.
    let replace = WriteOptions {
        create_parents: false,
        overwrite: true,
    };
    let mut w = vfs.open_write(&spec, &replace).unwrap();
    w.write_all(b"final").unwrap();
    w.flush().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"draft");
    w.commit().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"final");

    // Parents.
    let nested = dir.path().join("a").join("b").join("c.txt");
    let nested_spec = hikasa_core::Spec::Local(
        hikasa_core::LocalPath::new(
            dir.path()
                .canonicalize()
                .unwrap()
                .join("a")
                .join("b")
                .join("c.txt"),
        )
        .unwrap(),
    );
    assert!(matches!(
        vfs.open_write(&nested_spec, &WriteOptions::default()),
        Err(VfsError::NotFound { .. })
    ));
    let mut w = vfs
        .open_write(
            &nested_spec,
            &WriteOptions {
                create_parents: true,
                overwrite: false,
            },
        )
        .unwrap();
    w.write_all(b"deep").unwrap();
    w.commit().unwrap();
    assert_eq!(fs::read(&nested).unwrap(), b"deep");

    // Folders cannot be written.
    assert!(matches!(
        vfs.open_write(&lspec(dir.path()), &replace),
        Err(VfsError::IsADirectory { .. })
    ));
    // Two concurrent writers to the same target get distinct temporary files.
    let first = vfs
        .open_write(&lspec_new(&dir.path().join("twin")), &replace)
        .unwrap();
    let second = vfs
        .open_write(&lspec_new(&dir.path().join("twin")), &replace)
        .unwrap();
    assert_eq!(temp_files(dir.path()).len(), 2);
    first.commit().unwrap();
    second.commit().unwrap();
    assert!(temp_files(dir.path()).is_empty());
}

#[cfg(unix)]
mod unix {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::ffi::OsStr;
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    /// True when the tests run as root, which ignores permission bits.
    fn running_as_root(dir: &Path) -> bool {
        fs::metadata(dir).unwrap().uid() == 0
    }

    #[test]
    fn symlinks_and_a_broken_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), b"hello").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        symlink("a.txt", root.join("l_file")).unwrap();
        symlink("sub", root.join("l_dir")).unwrap();
        symlink("missing", root.join("l_broken")).unwrap();
        symlink("l_loop", root.join("l_loop")).unwrap();
        let vfs = LocalVfs::default();

        let plain = list(&vfs, root, &ListOptions::default());
        assert_eq!(
            names(&plain),
            ["a.txt", "l_broken", "l_dir", "l_file", "l_loop", "sub"]
        );
        assert!(plain.errors.is_empty());
        let l_file = &plain.entries[3];
        assert_eq!(l_file.kind, EntryKind::Symlink);
        assert_eq!(l_file.size, 5, "byte length of the target text");
        assert_eq!(l_file.link_target.as_deref(), Some("a.txt"));
        assert!(!l_file.followed);

        let followed = list(
            &vfs,
            root,
            &ListOptions {
                follow: FollowLinks::Yes,
                prefilter: None,
            },
        );
        assert_eq!(names(&followed), names(&plain));
        let by_name = |n: &str| {
            followed
                .entries
                .iter()
                .find(|e| e.name.as_str() == n)
                .unwrap()
        };
        let f = by_name("l_file");
        assert_eq!((f.kind, f.size, f.followed), (EntryKind::File, 5, true));
        assert_eq!(f.link_target.as_deref(), Some("a.txt"));
        assert_eq!(f.file_id, by_name("a.txt").file_id);
        let d = by_name("l_dir");
        assert_eq!((d.kind, d.followed), (EntryKind::Dir, true));
        assert_eq!(
            d.file_id,
            by_name("sub").file_id,
            "loop detection by FileId (D-33)"
        );
        let broken = by_name("l_broken");
        assert_eq!((broken.kind, broken.followed), (EntryKind::Symlink, false));
        let errors: Vec<(&str, ErrorKind)> = followed
            .errors
            .iter()
            .map(|e| (e.name.as_str(), e.error.kind))
            .collect();
        assert_eq!(errors[0], ("l_broken", ErrorKind::NotFound));
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].0, "l_loop");
        let broken_spec = root.canonicalize().unwrap().join("l_broken");
        assert_eq!(
            followed.errors[0].error.spec.as_deref(),
            Some(broken_spec.to_str().unwrap())
        );

        let link_spec = lspec_new(&root.join("l_broken"));
        assert_eq!(
            vfs.stat(&link_spec, FollowLinks::No).unwrap().kind,
            EntryKind::Symlink
        );
        assert!(matches!(
            vfs.stat(&link_spec, FollowLinks::Yes),
            Err(VfsError::NotFound { .. })
        ));
        let l_file_spec = lspec_new(&root.join("l_file"));
        let stat = vfs.stat(&l_file_spec, FollowLinks::Yes).unwrap();
        assert!(stat.followed);
        assert_eq!(stat.name.as_str(), "l_file");
        assert!(vfs.local_path(&l_file_spec).is_some(), "links to files map");
    }

    #[test]
    fn unix_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o4750)).unwrap();
        let meta = fs::metadata(&path).unwrap();
        let entry = LocalVfs::default()
            .stat(&lspec(&path), FollowLinks::No)
            .unwrap();
        let unix = entry.attrs.unix().unwrap();
        assert_eq!(unix.mode, 0o4750);
        assert_eq!((unix.uid, unix.gid), (meta.uid(), meta.gid()));
        assert_eq!(unix.permissions_string(entry.kind), "-rwsr-x---");
        assert_eq!(
            entry.file_id,
            Some(hikasa_vfs::FileId {
                dev: meta.dev(),
                ino: meta.ino()
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_names_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let raw = OsStr::from_bytes(b"bad\xff.txt");
        fs::write(dir.path().join(raw), b"raw").unwrap();
        let vfs = LocalVfs::default();
        let listing = list(&vfs, dir.path(), &ListOptions::default());
        let entry = &listing.entries[0];
        assert!(entry.name.is_lossy());
        assert_eq!(entry.name.as_str(), "bad\u{FFFD}.txt");
        assert_eq!(entry.name.to_os_string(), raw);
        let spec = vfs.child(&lspec(dir.path()), &entry.name).unwrap();
        assert!(spec.is_lossy());
        assert_eq!(vfs.stat(&spec, FollowLinks::No).unwrap().size, 3);
        assert_eq!(common::read_to_vec(vfs.open_read(&spec).unwrap()), b"raw");
    }

    #[test]
    fn permission_denied_folder() {
        let dir = tempfile::tempdir().unwrap();
        if running_as_root(dir.path()) {
            return; // root ignores permission bits
        }
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("x"), b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let vfs = LocalVfs::default();
        let err = vfs
            .list(
                &lspec(&locked),
                &ListOptions::default(),
                &CancelToken::never(),
            )
            .unwrap_err();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(err, VfsError::PermissionDenied { .. }), "{err:?}");
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    /// D-45 on the local file system: in a folder that can be read but not searched (`r--`), every `stat` of
    /// an entry fails. Without a prefilter each entry is an error; with a prefilter that declines everything
    /// the entries are name-only and no error appears, which proves that they were never stat'ed. Symlinks are
    /// still stat'ed (and fail here).
    #[test]
    fn prefilter_entries_are_never_stated() {
        let dir = tempfile::tempdir().unwrap();
        if running_as_root(dir.path()) {
            return;
        }
        let folder = dir.path().join("unsearchable");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("a.c"), b"").unwrap();
        fs::write(folder.join("b.h"), b"").unwrap();
        fs::create_dir(folder.join("obj")).unwrap();
        symlink("a.c", folder.join("link")).unwrap();
        let spec = lspec(&folder);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o444)).unwrap();
        let vfs = LocalVfs::default();
        let run = |opts: &ListOptions<'_>| {
            let mut listing = vfs.list(&spec, opts, &CancelToken::never()).unwrap();
            listing.sort_by_name();
            listing
        };
        let plain = run(&ListOptions::default());
        let seen = std::sync::Mutex::new(Vec::new());
        let decline = |name: &str, kind: EntryKind| {
            seen.lock().unwrap().push((name.to_owned(), kind));
            false
        };
        let filtered = run(&ListOptions {
            follow: FollowLinks::No,
            prefilter: Some(&decline),
        });
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            plain.errors.len(),
            4,
            "every stat fails in an unsearchable folder"
        );
        assert!(
            plain
                .errors
                .iter()
                .all(|e| e.error.kind == ErrorKind::PermissionDenied)
        );
        let name_only: Vec<(&str, EntryKind)> = filtered
            .name_only
            .iter()
            .map(|n| (n.name.as_str(), n.kind))
            .collect();
        assert_eq!(
            name_only,
            [
                ("a.c", EntryKind::File),
                ("b.h", EntryKind::File),
                ("obj", EntryKind::Dir)
            ]
        );
        assert!(filtered.entries.is_empty());
        let errors: Vec<&str> = filtered.errors.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(errors, ["link"], "symlinks are always stat'ed");
        let mut seen = seen.into_inner().unwrap();
        seen.sort();
        assert!(
            seen.iter().all(|(name, _)| name != "link"),
            "the prefilter never sees symlinks"
        );
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn overwrite_keeps_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("script.sh");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
        let vfs = LocalVfs::default();
        let mut w = vfs
            .open_write(
                &lspec(&target),
                &WriteOptions {
                    create_parents: false,
                    overwrite: true,
                },
            )
            .unwrap();
        w.write_all(b"new").unwrap();
        w.commit().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o750);
    }

    const REPLACE: WriteOptions = WriteOptions {
        create_parents: false,
        overwrite: true,
    };

    /// Writes `bytes` to `spec` with overwrite and commits.
    fn write_through(spec: &Spec, bytes: &[u8]) -> Result<(), VfsError> {
        let vfs = LocalVfs::default();
        let mut w = vfs.open_write(spec, &REPLACE)?;
        w.write_all(bytes).unwrap();
        w.commit()
    }

    /// Regression (wave-5 review, contract §1.2.7 item 12): a symbolic link target is resolved; the file it
    /// points to is replaced atomically (temporary file next to it) and the link stays, also through a chain of
    /// relative links and for a dangling link (the file it names is created).
    #[test]
    fn a_symlink_target_replaces_the_linked_file_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("r.html"), b"KEEP").unwrap();
        fs::set_permissions(real.join("r.html"), fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("rlink.html");
        symlink("real/r.html", &link).unwrap();
        write_through(&lspec_new(&link), b"report").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("real/r.html"));
        assert_eq!(fs::read(real.join("r.html")).unwrap(), b"report");
        assert_eq!(
            fs::metadata(real.join("r.html")).unwrap().mode() & 0o7777,
            0o640,
            "the linked file's permissions are kept"
        );
        assert!(temp_files(&real).is_empty() && temp_files(dir.path()).is_empty());
        // A chain: link2 → rlink.html → real/r.html.
        let link2 = dir.path().join("link2");
        symlink("rlink.html", &link2).unwrap();
        write_through(&lspec_new(&link2), b"second").unwrap();
        assert_eq!(fs::read(real.join("r.html")).unwrap(), b"second");
        assert!(
            fs::symlink_metadata(&link2)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // A dangling link creates the file it names.
        let dangling = dir.path().join("dangling");
        symlink("real/new.txt", &dangling).unwrap();
        write_through(&lspec_new(&dangling), b"new").unwrap();
        assert_eq!(fs::read(real.join("new.txt")).unwrap(), b"new");
        assert!(
            fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // A link to a folder is refused, the folder untouched.
        let to_dir = dir.path().join("to-dir");
        symlink("real", &to_dir).unwrap();
        assert!(matches!(
            write_through(&lspec_new(&to_dir), b"x"),
            Err(VfsError::IsADirectory { .. })
        ));
        // A loop is an error and replaces nothing.
        let (a, b) = (dir.path().join("loop-a"), dir.path().join("loop-b"));
        symlink("loop-b", &a).unwrap();
        symlink("loop-a", &b).unwrap();
        assert!(write_through(&lspec_new(&a), b"x").is_err());
        assert!(fs::symlink_metadata(&a).unwrap().file_type().is_symlink());
        // overwrite = false still refuses an existing link.
        let vfs = LocalVfs::default();
        assert!(matches!(
            vfs.open_write(&lspec_new(&link), &WriteOptions::default()),
            Err(VfsError::AlreadyExists { .. })
        ));
    }

    /// Regression (wave-5 review, contract §1.2.7 item 12): a FIFO target is written to directly and stays a
    /// FIFO (it used to be replaced by a regular file).
    #[test]
    fn a_fifo_target_is_written_to_and_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("myfifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: mkfifo is not available");
            return;
        }
        let reader = {
            let fifo = fifo.clone();
            std::thread::spawn(move || fs::read(fifo).unwrap())
        };
        write_through(&lspec_new(&fifo), b"through the pipe").unwrap();
        assert_eq!(reader.join().unwrap(), b"through the pipe");
        let meta = fs::symlink_metadata(&fifo).unwrap();
        assert!(
            !meta.is_file() && !meta.is_dir(),
            "still a FIFO: {:?}",
            meta.file_type()
        );
        assert!(temp_files(dir.path()).is_empty());
    }

    /// Regression (wave-5 review, contract §1.2.7 item 12): a character device such as `/dev/null` is written
    /// to directly (no temporary file in `/dev`, never renamed over, also as root).
    #[test]
    fn a_character_device_target_is_written_to_and_not_replaced() {
        let null = Path::new("/dev/null");
        let Ok(before) = fs::symlink_metadata(null) else {
            return;
        };
        write_through(
            &Spec::Local(hikasa_core::LocalPath::new(null.to_path_buf()).unwrap()),
            b"gone",
        )
        .unwrap();
        let after = fs::symlink_metadata(null).unwrap();
        assert!(!after.is_file(), "still a device");
        assert_eq!(before.rdev(), after.rdev());
        assert_eq!(before.ino(), after.ino(), "not replaced");
    }
}

/// Regression: symlink targets that are not UTF-8 keep their exact bytes (`link_target_raw`, like a lossy
/// `FileName`), so two different targets never look equal, and an unfollowed link's size is the byte length of
/// the exact target.
#[cfg(unix)]
#[test]
fn lossy_link_targets_are_exact() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let (l, r) = (dir.path().join("l"), dir.path().join("r"));
    fs::create_dir(&l).unwrap();
    fs::create_dir(&r).unwrap();
    std::os::unix::fs::symlink(OsStr::from_bytes(b"target-\xff"), l.join("ln")).unwrap();
    std::os::unix::fs::symlink(OsStr::from_bytes(b"target-\xfe"), r.join("ln")).unwrap();
    std::os::unix::fs::symlink(OsStr::from_bytes(b"\xff\xff\xff"), l.join("three")).unwrap();
    std::os::unix::fs::symlink("plain", l.join("plain")).unwrap();
    let vfs = LocalVfs::default();
    let stat = |p: &Path| vfs.stat(&lspec(p), FollowLinks::No).unwrap();
    let (el, er) = (stat(&l.join("ln")), stat(&r.join("ln")));
    assert_eq!(el.link_target, er.link_target, "the lossy texts are equal");
    assert_ne!(el, er, "the entries are not");
    assert_ne!(el.link_target_os(), er.link_target_os());
    assert_eq!(
        el.link_target_raw.as_deref(),
        Some(OsStr::from_bytes(b"target-\xff"))
    );
    assert_eq!(el.size, 8);
    let three = stat(&l.join("three"));
    assert_eq!(three.size, 3, "raw bytes, not the 9-byte lossy text");
    assert_eq!(
        three.link_target.as_deref(),
        Some("\u{FFFD}\u{FFFD}\u{FFFD}")
    );
    let plain = stat(&l.join("plain"));
    assert_eq!(plain.link_target.as_deref(), Some("plain"));
    assert_eq!(plain.link_target_raw, None, "raw only when lossy");
    assert_eq!(plain.size, plain.link_target.as_ref().unwrap().len() as u64);
    assert_eq!(plain.link_target_os().as_deref(), Some(OsStr::new("plain")));
    // Listings carry the same fields.
    let listing = list(&vfs, &l, &ListOptions::default());
    let ln = listing
        .entries
        .iter()
        .find(|e| e.name.as_str() == "ln")
        .unwrap();
    assert_eq!(ln, &el);
}

/// Regression: procfs reports size 0 for its links; the size is the byte length of the target read.
#[cfg(target_os = "linux")]
#[test]
fn proc_link_sizes_match_their_targets() {
    let path = Path::new("/proc/self/cwd");
    if fs::symlink_metadata(path).is_err() {
        return;
    }
    let vfs = LocalVfs::default();
    let spec = Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap());
    let entry = vfs.stat(&spec, FollowLinks::No).unwrap();
    assert_eq!(entry.kind, EntryKind::Symlink);
    let target = entry.link_target_os().unwrap();
    assert_eq!(entry.size, target.as_encoded_bytes().len() as u64);
    assert!(entry.size > 0);
}
