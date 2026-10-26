//! The router dispatch table (§1.2.4): every spec kind × every operation, the exact `Unsupported` messages
//! (never containing a password), the blob entries and `name_hint`.

mod common;

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use hikasa_core::spec::{PathStyle, SpecContext};
use hikasa_core::{CancelToken, Classify, ErrorKind, FileTime, LocalPath, Spec};
use hikasa_vfs::{
    Blob, Capabilities, EntryKind, FileName, FollowLinks, ListOptions, LocalVfs, Vfs, VfsError,
    VfsRouter, WriteOptions,
};

fn blob(bytes: &[u8], name: Option<&str>) -> Blob {
    Blob {
        bytes: Arc::from(bytes),
        mtime: FileTime::from_unix(1_700_000_000, 5).unwrap(),
        name: name.map(str::to_owned),
    }
}

fn parse(text: &str) -> Spec {
    let ctx = SpecContext {
        style: PathStyle::HOST,
        cwd: None,
        home: None,
    };
    Spec::parse(text, &ctx).unwrap()
}

fn message(result: Result<impl std::fmt::Debug, VfsError>) -> String {
    result.unwrap_err().to_string()
}

const BLOB_CAPS: Capabilities = Capabilities::READ
    .union(Capabilities::SEEK)
    .union(Capabilities::MTIME);

#[test]
fn local_specs_go_to_the_local_back_end() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    let router = VfsRouter::new(LocalVfs::default());
    let local = LocalVfs::default();
    let base = common::lspec(dir.path());
    let file = common::lspec(&dir.path().join("a.txt"));

    assert_eq!(
        router.stat(&file, FollowLinks::No).unwrap(),
        local.stat(&file, FollowLinks::No).unwrap()
    );
    assert_eq!(
        router.capabilities(&file).unwrap(),
        local.capabilities(&file).unwrap()
    );
    let listing = router
        .list(&base, &ListOptions::default(), &CancelToken::never())
        .unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(
        common::read_to_vec(router.open_read(&file).unwrap()),
        b"hello"
    );
    let child = router
        .child(&base, &FileName::new("b.txt").unwrap())
        .unwrap();
    let mut writer = router.open_write(&child, &WriteOptions::default()).unwrap();
    writer.write_all(b"out").unwrap();
    writer.commit().unwrap();
    assert_eq!(std::fs::read(dir.path().join("b.txt")).unwrap(), b"out");
    assert!(router.local_path(&file).is_some());
    assert!(router.local_path(&base).is_none(), "folders are not mapped");
    assert_eq!(router.name_hint(&file), None);
}

#[test]
fn stdin_without_a_blob() {
    let router = VfsRouter::new(LocalVfs::default());
    let stdin = Spec::Stdin;
    let unsupported = "-: stdin was not provided is not supported in this version";
    assert_eq!(message(router.stat(&stdin, FollowLinks::No)), unsupported);
    assert_eq!(message(router.open_read(&stdin).map(|_| ())), unsupported);
    assert_eq!(
        message(router.list(&stdin, &ListOptions::default(), &CancelToken::never())),
        "-: not a folder"
    );
    assert_eq!(
        message(
            router
                .open_write(&stdin, &WriteOptions::default())
                .map(|_| ())
        ),
        "-: writing to standard input is not supported in this version"
    );
    assert_eq!(
        message(router.child(&stdin, &FileName::new("x").unwrap())),
        "-: not a folder"
    );
    assert_eq!(router.capabilities(&stdin).unwrap(), BLOB_CAPS);
    assert_eq!(router.name_hint(&stdin), None);
    assert_eq!(router.local_path(&stdin), None);
    let err = router.stat(&stdin, FollowLinks::No).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[test]
fn stdin_with_a_blob() {
    let router = VfsRouter::new(LocalVfs::default()).with_stdin(blob(b"int main;\n", Some("x.c")));
    let entry = router.stat(&Spec::Stdin, FollowLinks::Yes).unwrap();
    assert_eq!(entry.name.as_str(), "-");
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, 10);
    assert_eq!(entry.mtime, FileTime::from_unix(1_700_000_000, 5));
    assert_eq!(entry.ctime, None);
    assert!(!entry.followed);
    let mut reader = router.open_read(&Spec::Stdin).unwrap();
    assert_eq!(common::read_to_vec(&mut reader), b"int main;\n");
    reader.seek(SeekFrom::Start(4)).unwrap();
    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "main;\n");
    assert_eq!(router.name_hint(&Spec::Stdin), Some("x.c"));
    assert_eq!(router.name_hint(&Spec::Clipboard), None);
    assert_eq!(router.capabilities(&Spec::Stdin).unwrap(), BLOB_CAPS);
    assert_eq!(
        message(router.list(&Spec::Stdin, &ListOptions::default(), &CancelToken::never())),
        "-: not a folder"
    );
    assert_eq!(
        message(
            router
                .open_write(&Spec::Stdin, &WriteOptions::default())
                .map(|_| ())
        ),
        "-: writing to standard input is not supported in this version"
    );
    // A blob without a name hint.
    let unnamed = VfsRouter::new(LocalVfs::default()).with_stdin(blob(b"", None));
    assert_eq!(unnamed.name_hint(&Spec::Stdin), None);
    assert_eq!(unnamed.stat(&Spec::Stdin, FollowLinks::No).unwrap().size, 0);
}

#[test]
fn clipboard_with_and_without_a_blob() {
    let bare = VfsRouter::new(LocalVfs::default());
    let clip = Spec::Clipboard;
    let unsupported = "clipboard://: clipboard access is not supported in this version";
    assert_eq!(message(bare.stat(&clip, FollowLinks::No)), unsupported);
    assert_eq!(message(bare.open_read(&clip).map(|_| ())), unsupported);
    assert_eq!(
        message(bare.list(&clip, &ListOptions::default(), &CancelToken::never())),
        "clipboard://: not a folder"
    );
    assert_eq!(
        message(bare.open_write(&clip, &WriteOptions::default()).map(|_| ())),
        "clipboard://: writing to the clipboard is not supported in this version"
    );
    assert_eq!(bare.capabilities(&clip).unwrap(), BLOB_CAPS);

    let router = bare.with_clipboard(blob(b"copied", Some("notes.md")));
    let entry = router.stat(&clip, FollowLinks::No).unwrap();
    assert_eq!(entry.name.as_str(), "clipboard://");
    assert_eq!(entry.size, 6);
    assert_eq!(
        common::read_to_vec(router.open_read(&clip).unwrap()),
        b"copied"
    );
    assert_eq!(router.name_hint(&clip), Some("notes.md"));
    assert_eq!(router.name_hint(&Spec::Stdin), None);
    assert_eq!(
        message(router.child(&clip, &FileName::new("x").unwrap())),
        "clipboard://: not a folder"
    );
}

/// Every unsupported spec kind fails every operation with `Unsupported{what: "<scheme> specs"}`.
#[test]
fn unsupported_schemes() {
    let router = VfsRouter::new(LocalVfs::default());
    let cases = [
        ("ftp://user:secret@Host/p", "ftp://user@host/p", "ftp specs"),
        ("ftps://h/p", "ftps://h/p", "ftps specs"),
        ("sftp://u@h:2222/p", "sftp://u@h:2222/p", "sftp specs"),
        ("dav://h/p", "dav://h/p", "dav specs"),
        ("davs://h/p", "davs://h/p", "davs specs"),
        ("s3://bucket/prefix/", "s3://bucket/prefix/", "s3 specs"),
        ("svn://h/r", "svn://h/r", "svn specs"),
        ("svn+http://h/r", "svn+http://h/r", "svn+http specs"),
        ("svn+https://h/r", "svn+https://h/r", "svn+https specs"),
        ("svn+ssh://h/r", "svn+ssh://h/r", "svn+ssh specs"),
        ("mtp://Phone/DCIM", "mtp://phone/DCIM", "mtp specs"),
        (
            "profile:My Prof?dir/f.txt",
            "profile:My Prof?dir/f.txt",
            "profile specs",
        ),
        ("http://example.com/a", "http://example.com/a", "http specs"),
        (
            "https://u:pw@Example.com/a?b",
            "https://u@example.com/a?b",
            "https specs",
        ),
    ];
    for (input, canonical, what) in cases {
        let spec = parse(input);
        let expected = format!("{canonical}: {what} is not supported in this version");
        assert!(!expected.contains("secret") && !expected.contains("pw@"));
        assert_eq!(
            message(router.stat(&spec, FollowLinks::No)),
            expected,
            "{input}"
        );
        assert_eq!(message(router.open_read(&spec).map(|_| ())), expected);
        assert_eq!(
            message(router.list(&spec, &ListOptions::default(), &CancelToken::never())),
            expected
        );
        assert_eq!(
            message(
                router
                    .open_write(&spec, &WriteOptions::default())
                    .map(|_| ())
            ),
            expected
        );
        assert_eq!(message(router.capabilities(&spec)), expected);
        assert_eq!(
            message(router.child(&spec, &FileName::new("x").unwrap())),
            expected
        );
        assert_eq!(router.local_path(&spec), None);
        assert_eq!(router.name_hint(&spec), None);
        let err = router.stat(&spec, FollowLinks::No).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        assert!(matches!(err, VfsError::Unsupported { what: w, .. } if w == what));
    }

    let base = std::env::temp_dir().canonicalize().unwrap();
    let local = LocalPath::new(base.join("a.zip")).unwrap();
    let archive = Spec::ArchiveMember {
        archive: Box::new(Spec::Local(local.clone())),
        member: "inner/f.txt".into(),
    };
    let snapshot = Spec::Snapshot {
        file: local,
        member: "dir".into(),
    };
    for (spec, what) in [(archive, "archive specs"), (snapshot, "snapshot specs")] {
        let expected = format!(
            "{}: {what} is not supported in this version",
            spec.canonical()
        );
        assert_eq!(message(router.stat(&spec, FollowLinks::No)), expected);
        assert_eq!(message(router.open_read(&spec).map(|_| ())), expected);
        assert_eq!(message(router.capabilities(&spec)), expected);
        assert_eq!(
            message(router.list(&spec, &ListOptions::default(), &CancelToken::never())),
            expected
        );
        assert_eq!(
            message(
                router
                    .open_write(&spec, &WriteOptions::default())
                    .map(|_| ())
            ),
            expected
        );
    }
}

#[test]
fn blobs_do_not_leak_their_bytes_in_debug_output() {
    let router = VfsRouter::new(LocalVfs::default()).with_stdin(blob(b"secret bytes", None));
    let debug = format!("{router:?}");
    assert!(debug.contains("len: 12"), "{debug}");
    assert!(!debug.contains("115, 101"), "{debug}");
    assert!(router.local().options().case.is_none());
}
