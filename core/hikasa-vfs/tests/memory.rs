//! `MemoryVfs`: tree semantics, symlinks and loops, `FollowLinks`, the prefilter (D-45, proven without any
//! `stat`), listing orders (INV-VFS-1), atomic writes (INV-VFS-3), case-insensitive lookups and fault injection
//! (every `VfsOp` × `ErrorKind` combination surfaces the documented error).

mod common;

use std::io::{Read, Seek, SeekFrom, Write};

use common::{host, mspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Classify, ErrorKind, FileTime, Spec, io_error_kind,
};
use hikasa_vfs::{
    Attrs, Capabilities, DosAttrs, EntryKind, Fault, FileName, FollowLinks, ListOptions,
    ListingOrder, MemMeta, MemoryVfs, UnixMeta, Vfs, VfsError, VfsOp, WriteOptions,
};

fn t(secs: i64) -> Option<FileTime> {
    FileTime::from_unix(secs, 0)
}

/// /src/{a.c, b.h, sub/{x.c}}, /docs/readme.md, /link_to_src → ../src (relative), /abs_link → /docs,
/// /broken → missing, /loop_a ↔ /loop_b, /src/sub/up → /src (a folder cycle).
fn sample() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = MemMeta {
        mtime: t(1_000),
        ctime: t(500),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: 0o644,
                uid: 1000,
                gid: 100,
            },
        }),
        crc32: None,
    };
    vfs.insert_file(&host("/src/a.c"), b"int a;\n".to_vec(), meta.clone())
        .unwrap();
    vfs.insert_file(
        &host("/src/b.h"),
        b"#pragma once\n".to_vec(),
        MemMeta::default(),
    )
    .unwrap();
    vfs.insert_file(
        &host("/src/sub/x.c"),
        b"x".to_vec(),
        MemMeta {
            crc32: Some(0xDEAD_BEEF),
            ..MemMeta::default()
        },
    )
    .unwrap();
    vfs.insert_file(
        &host("/docs/readme.md"),
        b"# hi\n".to_vec(),
        MemMeta::default(),
    )
    .unwrap();
    vfs.insert_symlink(&host("/link_to_src"), "src", MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/abs_link"), &host("/docs"), MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/broken"), "missing", MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/loop_a"), "loop_b", MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/loop_b"), "loop_a", MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/src/sub/up"), "../../src", MemMeta::default())
        .unwrap();
    vfs
}

fn list(vfs: &dyn Vfs, path: &str, follow: FollowLinks) -> hikasa_vfs::Listing {
    let opts = ListOptions {
        follow,
        prefilter: None,
    };
    let mut listing = vfs
        .list(&mspec(path), &opts, &CancelToken::never())
        .unwrap();
    listing.sort_by_name();
    listing
}

fn names(listing: &hikasa_vfs::Listing) -> Vec<String> {
    listing
        .entries
        .iter()
        .map(|e| e.name.as_str().to_owned())
        .collect()
}

#[test]
fn stat_files_folders_and_metadata() {
    let vfs = sample();
    let a = vfs.stat(&mspec("/src/a.c"), FollowLinks::No).unwrap();
    assert_eq!(a.name.as_str(), "a.c");
    assert_eq!(a.kind, EntryKind::File);
    assert_eq!(a.size, 7);
    assert_eq!(a.mtime, t(1_000));
    assert_eq!(a.ctime, t(500));
    assert_eq!(a.attrs.unix().unwrap().mode, 0o644);
    assert!(!a.followed);
    assert!(a.file_id.is_some());
    let src = vfs.stat(&mspec("/src"), FollowLinks::No).unwrap();
    assert_eq!((src.kind, src.size), (EntryKind::Dir, 0));
    assert_eq!(src.attrs, Attrs::None);
    let x = vfs.stat(&mspec("/src/sub/x.c"), FollowLinks::No).unwrap();
    assert_eq!(x.crc32, Some(0xDEAD_BEEF));
    let root = vfs.stat(&mspec("/"), FollowLinks::No).unwrap();
    assert_eq!(root.kind, EntryKind::Dir);
    assert_eq!(root.name.as_str(), host("/"));
    // Missing entries, and paths through a file.
    assert!(matches!(
        vfs.stat(&mspec("/nope"), FollowLinks::No),
        Err(VfsError::NotFound { .. })
    ));
    assert!(matches!(
        vfs.stat(&mspec("/src/a.c/x"), FollowLinks::No),
        Err(VfsError::NotADirectory { .. })
    ));
    assert_eq!(
        vfs.stat(&mspec("/nope"), FollowLinks::No)
            .unwrap_err()
            .to_string(),
        format!("{}: not found", host("/nope"))
    );
}

#[test]
fn symlinks_follow_and_report() {
    let vfs = sample();
    let link = vfs.stat(&mspec("/link_to_src"), FollowLinks::No).unwrap();
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.size, 3, "length of the target text");
    assert_eq!(link.link_target.as_deref(), Some("src"));
    assert!(!link.followed);
    let followed = vfs.stat(&mspec("/link_to_src"), FollowLinks::Yes).unwrap();
    assert_eq!(followed.kind, EntryKind::Dir);
    assert_eq!(followed.name.as_str(), "link_to_src");
    assert_eq!(followed.link_target.as_deref(), Some("src"));
    assert!(followed.followed);
    let src = vfs.stat(&mspec("/src"), FollowLinks::No).unwrap();
    assert_eq!(
        followed.file_id, src.file_id,
        "a followed link has the target's id"
    );
    // Intermediate links are always followed.
    let a = vfs
        .stat(&mspec("/link_to_src/a.c"), FollowLinks::No)
        .unwrap();
    assert_eq!(a.size, 7);
    let readme = vfs
        .stat(&mspec("/abs_link/readme.md"), FollowLinks::No)
        .unwrap();
    assert_eq!(readme.size, 5);
    // Broken links and loops.
    assert_eq!(
        vfs.stat(&mspec("/broken"), FollowLinks::No).unwrap().kind,
        EntryKind::Symlink
    );
    assert!(matches!(
        vfs.stat(&mspec("/broken"), FollowLinks::Yes),
        Err(VfsError::NotFound { .. })
    ));
    let loop_err = vfs.stat(&mspec("/loop_a"), FollowLinks::Yes).unwrap_err();
    assert_eq!(loop_err.kind(), ErrorKind::Io);
    assert!(
        loop_err
            .to_string()
            .to_lowercase()
            .contains("too many levels of symbolic links")
            || cfg!(windows),
        "{loop_err}"
    );
    // A folder cycle resolves; the file id reveals the loop (D-33).
    let up = vfs.stat(&mspec("/src/sub/up"), FollowLinks::Yes).unwrap();
    assert_eq!(up.file_id, src.file_id);
    let deep = vfs
        .stat(&mspec("/src/sub/up/sub/up/sub/x.c"), FollowLinks::No)
        .unwrap();
    assert_eq!(deep.size, 1);
    // open_read follows links.
    assert_eq!(
        common::read_to_vec(vfs.open_read(&mspec("/link_to_src/a.c")).unwrap()),
        b"int a;\n"
    );
}

#[test]
fn listings_with_and_without_following() {
    let vfs = sample();
    let root = list(&vfs, "/", FollowLinks::No);
    assert_eq!(
        names(&root),
        [
            "abs_link",
            "broken",
            "docs",
            "link_to_src",
            "loop_a",
            "loop_b",
            "src"
        ]
    );
    assert!(root.errors.is_empty());
    assert!(root.name_only.is_empty());
    let kinds: Vec<EntryKind> = root.entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EntryKind::Symlink,
            EntryKind::Symlink,
            EntryKind::Dir,
            EntryKind::Symlink,
            EntryKind::Symlink,
            EntryKind::Symlink,
            EntryKind::Dir
        ]
    );
    // Following: broken links and loops are listed unfollowed AND reported as errors.
    let followed = list(&vfs, "/", FollowLinks::Yes);
    assert_eq!(names(&followed), names(&root));
    let by_name = |n: &str| {
        followed
            .entries
            .iter()
            .find(|e| e.name.as_str() == n)
            .unwrap()
    };
    assert!(by_name("link_to_src").followed);
    assert_eq!(by_name("link_to_src").kind, EntryKind::Dir);
    assert!(by_name("abs_link").followed);
    assert!(!by_name("broken").followed);
    assert_eq!(by_name("broken").kind, EntryKind::Symlink);
    let errors: Vec<(&str, ErrorKind)> = followed
        .errors
        .iter()
        .map(|e| (e.name.as_str(), e.error.kind))
        .collect();
    assert_eq!(
        errors,
        [
            ("broken", ErrorKind::NotFound),
            ("loop_a", ErrorKind::Io),
            ("loop_b", ErrorKind::Io)
        ]
    );
    assert_eq!(
        followed.errors[0].error.spec.as_deref(),
        Some(host("/broken").as_str())
    );
    // Listing through a link, and listing a file.
    assert_eq!(
        names(&list(&vfs, "/link_to_src", FollowLinks::No)),
        ["a.c", "b.h", "sub"]
    );
    assert!(matches!(
        vfs.list(
            &mspec("/src/a.c"),
            &ListOptions::default(),
            &CancelToken::never()
        ),
        Err(VfsError::NotADirectory { .. })
    ));
    assert!(matches!(
        vfs.list(
            &mspec("/missing"),
            &ListOptions::default(),
            &CancelToken::never()
        ),
        Err(VfsError::NotFound { .. })
    ));
    // Cancellation.
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        vfs.list(&mspec("/"), &ListOptions::default(), &token),
        Err(VfsError::Cancelled)
    ));
}

/// D-45: declined entries are name-only and never stat'ed; symlinks are always stat'ed and never passed to
/// the prefilter.
#[test]
fn prefilter_reports_name_only_without_stat() {
    let vfs = sample();
    let counting = common::CountingVfs::new(&vfs);
    let seen = std::sync::Mutex::new(Vec::new());
    let decline_all = |name: &str, kind: EntryKind| {
        seen.lock().unwrap().push((name.to_owned(), kind));
        false
    };
    // A stat fault on a declined entry would surface as an error if it were stat'ed.
    vfs.inject(Fault {
        path: host("/docs"),
        op: VfsOp::Stat,
        kind: ErrorKind::PermissionDenied,
    });
    let before = vfs.stat_calls();
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&decline_all),
    };
    let mut listing = counting
        .list(&mspec("/"), &opts, &CancelToken::never())
        .unwrap();
    listing.sort_by_name();
    let name_only: Vec<(&str, EntryKind)> = listing
        .name_only
        .iter()
        .map(|n| (n.name.as_str(), n.kind))
        .collect();
    assert_eq!(
        name_only,
        [("docs", EntryKind::Dir), ("src", EntryKind::Dir)]
    );
    assert!(
        listing.errors.is_empty(),
        "declined entries are not stat'ed"
    );
    assert_eq!(
        names(&listing),
        ["abs_link", "broken", "link_to_src", "loop_a", "loop_b"],
        "symlinks are always stat'ed"
    );
    assert_eq!(counting.stats(), 0, "no stat through the Vfs");
    assert_eq!(
        vfs.stat_calls() - before,
        5,
        "only the five symlinks were stat'ed"
    );
    let mut seen = seen.into_inner().unwrap();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("docs".to_owned(), EntryKind::Dir),
            ("src".to_owned(), EntryKind::Dir)
        ],
        "the prefilter never sees symlinks"
    );
    // Accepted entries are stat'ed (and the injected fault fires).
    let accept = |_: &str, _: EntryKind| true;
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&accept),
    };
    let listing = vfs.list(&mspec("/"), &opts, &CancelToken::never()).unwrap();
    assert!(listing.name_only.is_empty());
    assert_eq!(listing.errors.len(), 1);
    assert_eq!(listing.errors[0].name.as_str(), "docs");
    assert_eq!(listing.errors[0].error.kind, ErrorKind::PermissionDenied);
    // Files are passed with their kind.
    let files_only = |_: &str, kind: EntryKind| kind == EntryKind::Dir;
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&files_only),
    };
    let mut src = vfs
        .list(&mspec("/src"), &opts, &CancelToken::never())
        .unwrap();
    src.sort_by_name();
    assert_eq!(names(&src), ["sub"]);
    assert_eq!(src.name_only.len(), 2);
    assert!(src.name_only.iter().all(|n| n.kind == EntryKind::File));
}

#[test]
fn listing_orders_are_permutations() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for n in ["b", "a", "d", "c", "B", "é", "z"] {
        vfs.insert_file(&host(&format!("/d/{n}")), b"".to_vec(), MemMeta::default())
            .unwrap();
    }
    let raw = |order| {
        vfs.set_listing_order(order);
        vfs.list(&mspec("/d"), &ListOptions::default(), &CancelToken::never())
            .unwrap()
            .entries
            .iter()
            .map(|e| e.name.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    let sorted = raw(ListingOrder::Sorted);
    assert_eq!(sorted, ["B", "a", "b", "c", "d", "z", "é"], "byte order");
    let mut reversed = sorted.clone();
    reversed.reverse();
    assert_eq!(raw(ListingOrder::Reversed), reversed);
    let shuffled = raw(ListingOrder::Shuffled { seed: 7 });
    assert_eq!(
        shuffled,
        raw(ListingOrder::Shuffled { seed: 7 }),
        "deterministic"
    );
    assert_ne!(shuffled, sorted);
    let mut again = shuffled.clone();
    again.sort();
    assert_eq!(again, sorted);
}

#[test]
fn case_insensitive_lookups_keep_the_last_spelling() {
    let vfs = MemoryVfs::new(CaseSensitivity::Insensitive);
    vfs.insert_file(&host("/Dir/File.TXT"), b"1".to_vec(), MemMeta::default())
        .unwrap();
    assert_eq!(
        vfs.stat(&mspec("/dir/file.txt"), FollowLinks::No)
            .unwrap()
            .size,
        1
    );
    vfs.insert_file(&host("/DIR/file.txt"), b"22".to_vec(), MemMeta::default())
        .unwrap();
    let listing = list(&vfs, "/dir", FollowLinks::No);
    assert_eq!(names(&listing), ["file.txt"]);
    assert_eq!(listing.entries[0].size, 2);
    assert!(
        !vfs.capabilities(&mspec("/dir"))
            .unwrap()
            .contains(Capabilities::CASE_SENSITIVE)
    );
    let sensitive = MemoryVfs::default();
    sensitive
        .insert_file(&host("/A"), b"".to_vec(), MemMeta::default())
        .unwrap();
    assert!(sensitive.stat(&mspec("/a"), FollowLinks::No).is_err());
    assert!(
        sensitive
            .capabilities(&mspec("/"))
            .unwrap()
            .contains(Capabilities::CASE_SENSITIVE)
    );
}

#[test]
fn inserts_validate_and_replace() {
    let vfs = MemoryVfs::default();
    assert!(matches!(
        vfs.insert_file("relative/x", b"".to_vec(), MemMeta::default()),
        Err(VfsError::InvalidName { .. })
    ));
    vfs.insert_file(&host("/f"), b"1".to_vec(), MemMeta::default())
        .unwrap();
    // A file cannot become a folder, nor hold children.
    assert!(matches!(
        vfs.insert_dir(&host("/f"), MemMeta::default()),
        Err(VfsError::AlreadyExists { .. })
    ));
    assert!(matches!(
        vfs.insert_file(&host("/f/g"), b"".to_vec(), MemMeta::default()),
        Err(VfsError::NotADirectory { .. })
    ));
    // Replacing a file keeps it a file with the new content and metadata.
    let dos = MemMeta {
        attrs: Some(Attrs::Dos {
            attrs: DosAttrs::HIDDEN | DosAttrs::READONLY,
        }),
        ..MemMeta::default()
    };
    vfs.insert_file(&host("/f"), b"22".to_vec(), dos).unwrap();
    let f = vfs.stat(&mspec("/f"), FollowLinks::No).unwrap();
    assert_eq!(f.size, 2);
    assert_eq!(f.attrs.dos().unwrap().letters(), "HR");
    // insert_dir on an existing folder updates its metadata and keeps the children.
    vfs.insert_file(&host("/d/c"), b"".to_vec(), MemMeta::default())
        .unwrap();
    vfs.insert_dir(
        &host("/d"),
        MemMeta {
            mtime: t(9),
            ..MemMeta::default()
        },
    )
    .unwrap();
    assert_eq!(vfs.stat(&mspec("/d"), FollowLinks::No).unwrap().mtime, t(9));
    assert_eq!(names(&list(&vfs, "/d", FollowLinks::No)), ["c"]);
    assert!(matches!(
        vfs.insert_file(&host("/d"), b"".to_vec(), MemMeta::default()),
        Err(VfsError::AlreadyExists { .. })
    ));
    assert_eq!(vfs.read_back(&host("/f")).as_deref(), Some(&b"22"[..]));
    assert_eq!(vfs.read_back(&host("/d")), None);
    assert!(format!("{vfs:?}").starts_with("MemoryVfs"));
}

#[test]
fn open_read_seeks_and_rejects_folders() {
    let vfs = sample();
    let mut r = vfs.open_read(&mspec("/src/b.h")).unwrap();
    r.seek(SeekFrom::Start(1)).unwrap();
    let mut s = String::new();
    r.read_to_string(&mut s).unwrap();
    assert_eq!(s, "pragma once\n");
    assert!(matches!(
        vfs.open_read(&mspec("/src")),
        Err(VfsError::IsADirectory { .. })
    ));
    assert!(matches!(
        vfs.open_read(&mspec("/broken")),
        Err(VfsError::NotFound { .. })
    ));
}

/// INV-VFS-3: nothing is visible before `commit`; dropping discards; `overwrite = false` gives
/// `AlreadyExists`; parents are created on request.
#[test]
fn atomic_writes() {
    let vfs = sample();
    let target = mspec("/out/report.txt");
    assert!(matches!(
        vfs.open_write(&target, &WriteOptions::default()),
        Err(VfsError::NotFound { .. })
    ));
    let create = WriteOptions {
        create_parents: true,
        overwrite: false,
    };
    let mut w = vfs.open_write(&target, &create).unwrap();
    w.write_all(b"hello ").unwrap();
    w.write_all(b"world").unwrap();
    assert!(
        vfs.stat(&target, FollowLinks::No).is_err(),
        "invisible before commit"
    );
    w.commit().unwrap();
    assert_eq!(
        vfs.read_back(&host("/out/report.txt")).as_deref(),
        Some(&b"hello world"[..])
    );
    let entry = vfs.stat(&target, FollowLinks::No).unwrap();
    assert_eq!((entry.size, entry.mtime), (11, None));
    // Drop discards.
    let other = mspec("/out/dropped.txt");
    let mut w = vfs.open_write(&other, &create).unwrap();
    w.write_all(b"x").unwrap();
    drop(w);
    assert!(vfs.stat(&other, FollowLinks::No).is_err());
    // No overwrite.
    assert!(matches!(
        vfs.open_write(&target, &create),
        Err(VfsError::AlreadyExists { .. })
    ));
    // A racing creation between open and commit is caught at commit.
    let late = mspec("/out/late.txt");
    let w = vfs.open_write(&late, &create).unwrap();
    vfs.insert_file(
        &host("/out/late.txt"),
        b"first".to_vec(),
        MemMeta::default(),
    )
    .unwrap();
    assert!(matches!(w.commit(), Err(VfsError::AlreadyExists { .. })));
    assert_eq!(
        vfs.read_back(&host("/out/late.txt")).as_deref(),
        Some(&b"first"[..])
    );
    // Overwrite keeps times and attributes, drops the CRC.
    vfs.insert_file(
        &host("/out/keep.txt"),
        b"old".to_vec(),
        MemMeta {
            mtime: t(5),
            crc32: Some(1),
            ..MemMeta::default()
        },
    )
    .unwrap();
    let overwrite = WriteOptions {
        create_parents: false,
        overwrite: true,
    };
    let mut w = vfs.open_write(&mspec("/out/keep.txt"), &overwrite).unwrap();
    w.write_all(b"new!").unwrap();
    w.commit().unwrap();
    let kept = vfs.stat(&mspec("/out/keep.txt"), FollowLinks::No).unwrap();
    assert_eq!((kept.size, kept.mtime, kept.crc32), (4, t(5), None));
    // Folders cannot be written; writing through a parent link works.
    assert!(matches!(
        vfs.open_write(&mspec("/src"), &overwrite),
        Err(VfsError::IsADirectory { .. })
    ));
    let mut w = vfs
        .open_write(&mspec("/link_to_src/new.c"), &WriteOptions::default())
        .unwrap();
    w.write_all(b"n").unwrap();
    w.commit().unwrap();
    assert_eq!(
        vfs.read_back(&host("/src/new.c")).as_deref(),
        Some(&b"n"[..])
    );
    // A parent that is a file.
    assert!(matches!(
        vfs.open_write(&mspec("/src/a.c/x"), &create),
        Err(VfsError::NotADirectory { .. })
    ));
}

#[test]
fn children_and_capabilities() {
    let vfs = sample();
    let child = vfs
        .child(&mspec("/src"), &FileName::new("a.c").unwrap())
        .unwrap();
    assert_eq!(child, mspec("/src/a.c"));
    assert_eq!(vfs.local_path(&child), None, "MemoryVfs never maps");
    let caps = vfs.capabilities(&mspec("/")).unwrap();
    assert!(
        caps.contains(Capabilities::READ | Capabilities::SYMLINKS | Capabilities::CRC_PROVIDED)
    );
    assert!(!caps.contains(Capabilities::LOCAL_PATH));
    vfs.set_capabilities(Capabilities::READ);
    assert_eq!(vfs.capabilities(&mspec("/")).unwrap(), Capabilities::READ);
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
}

const KINDS: [ErrorKind; 13] = [
    ErrorKind::NotFound,
    ErrorKind::PermissionDenied,
    ErrorKind::AlreadyExists,
    ErrorKind::NotADirectory,
    ErrorKind::IsADirectory,
    ErrorKind::InvalidInput,
    ErrorKind::InvalidData,
    ErrorKind::Unsupported,
    ErrorKind::Cancelled,
    ErrorKind::Io,
    ErrorKind::TooLarge,
    ErrorKind::Protocol,
    ErrorKind::Internal,
];

/// The documented classification of an injected fault: the fault's kind, except protocol and internal
/// (no file system error carries them), which classify as io.
fn expected_kind(kind: ErrorKind) -> ErrorKind {
    match kind {
        ErrorKind::Protocol | ErrorKind::Internal => ErrorKind::Io,
        other => other,
    }
}

/// Every `VfsOp` × `ErrorKind` combination surfaces the documented error (§5.5 rule 3, §6.2.2).
#[test]
fn fault_injection_matrix() {
    let file = "/data/f.txt";
    let dir = "/data";
    for op in VfsOp::ALL {
        for kind in KINDS {
            let vfs = MemoryVfs::default();
            vfs.insert_file(&host(file), b"content".to_vec(), MemMeta::default())
                .unwrap();
            let path = match op {
                VfsOp::List => dir,
                _ => file,
            };
            vfs.inject(Fault {
                path: host(path),
                op,
                kind,
            });
            let want = expected_kind(kind);
            let context = format!("{op:?} × {kind:?}");
            match op {
                VfsOp::Stat => {
                    let err = vfs.stat(&mspec(file), FollowLinks::No).unwrap_err();
                    assert_eq!(err.kind(), want, "{context}");
                    // Listing the parent reports an entry error.
                    let listing = vfs
                        .list(&mspec(dir), &ListOptions::default(), &CancelToken::never())
                        .unwrap();
                    assert!(listing.entries.is_empty(), "{context}");
                    assert_eq!(listing.errors.len(), 1, "{context}");
                    assert_eq!(listing.errors[0].error.kind, want, "{context}");
                    assert_eq!(listing.errors[0].name.as_str(), "f.txt");
                }
                VfsOp::List => {
                    let err = vfs
                        .list(&mspec(dir), &ListOptions::default(), &CancelToken::never())
                        .unwrap_err();
                    assert_eq!(err.kind(), want, "{context}");
                }
                VfsOp::Open => {
                    let err = vfs.open_read(&mspec(file)).map(|_| ()).unwrap_err();
                    assert_eq!(err.kind(), want, "{context}");
                }
                VfsOp::Read => {
                    let mut reader = vfs.open_read(&mspec(file)).unwrap();
                    let mut buf = [0u8; 4];
                    let err = reader.read(&mut buf).unwrap_err();
                    assert_eq!(io_error_kind(&err), want, "{context}");
                }
                VfsOp::Write => {
                    let opts = WriteOptions {
                        create_parents: false,
                        overwrite: true,
                    };
                    let err = vfs.open_write(&mspec(file), &opts).map(|_| ()).unwrap_err();
                    assert_eq!(err.kind(), want, "{context}");
                }
                VfsOp::Commit => {
                    let opts = WriteOptions {
                        create_parents: false,
                        overwrite: true,
                    };
                    let mut w = vfs.open_write(&mspec(file), &opts).unwrap();
                    w.write_all(b"new").unwrap();
                    let err = w.commit().unwrap_err();
                    assert_eq!(err.kind(), want, "{context}");
                    assert_eq!(
                        vfs.read_back(&host(file)).as_deref(),
                        Some(&b"content"[..]),
                        "nothing written ({context})"
                    );
                }
                VfsOp::Map => {
                    // MemoryVfs never maps: the fault has no effect anywhere.
                    assert_eq!(vfs.stat(&mspec(file), FollowLinks::No).unwrap().size, 7);
                    assert_eq!(
                        common::read_to_vec(vfs.open_read(&mspec(file)).unwrap()),
                        b"content"
                    );
                }
            }
            // Dedicated variants for the file-system kinds, `Cancelled` for cancellation.
            if op == VfsOp::Open {
                let err = vfs.open_read(&mspec(file)).map(|_| ()).unwrap_err();
                let dedicated = match kind {
                    ErrorKind::NotFound => matches!(err, VfsError::NotFound { .. }),
                    ErrorKind::PermissionDenied => matches!(err, VfsError::PermissionDenied { .. }),
                    ErrorKind::AlreadyExists => matches!(err, VfsError::AlreadyExists { .. }),
                    ErrorKind::NotADirectory => matches!(err, VfsError::NotADirectory { .. }),
                    ErrorKind::IsADirectory => matches!(err, VfsError::IsADirectory { .. }),
                    ErrorKind::Cancelled => matches!(err, VfsError::Cancelled),
                    _ => matches!(
                        err,
                        VfsError::Io {
                            op: VfsOp::Open,
                            ..
                        }
                    ),
                };
                assert!(dedicated, "{context}: {err:?}");
            }
        }
    }
}

#[test]
fn faults_match_lexically_and_can_be_cleared() {
    let vfs = MemoryVfs::new(CaseSensitivity::Insensitive);
    vfs.insert_file(&host("/A/b.txt"), b"x".to_vec(), MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/link"), "A", MemMeta::default())
        .unwrap();
    vfs.inject(Fault {
        path: host("/a/B.TXT"),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    vfs.inject(Fault {
        path: "not absolute".into(),
        op: VfsOp::Open,
        kind: ErrorKind::NotFound,
    });
    assert!(matches!(
        vfs.open_read(&mspec("/A/b.txt")),
        Err(VfsError::PermissionDenied { .. })
    ));
    // The fault names the path as given, not the resolved one.
    assert!(vfs.open_read(&mspec("/link/b.txt")).is_ok());
    vfs.clear_faults();
    assert!(vfs.open_read(&mspec("/A/b.txt")).is_ok());
}

/// The `std::io::Error` inside a `VfsError::Io`.
fn io_source(error: &VfsError) -> Option<&std::io::Error> {
    std::error::Error::source(error).and_then(|s| s.downcast_ref::<std::io::Error>())
}

/// Regression (fault fidelity, §5.5 rule 3): a symlink loop gives the same error on `MemoryVfs` as on the local
/// file system — the same `std::io::ErrorKind` and the same message.
#[cfg(unix)]
#[test]
fn link_loops_match_the_local_file_system() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("loop");
    std::os::unix::fs::symlink("loop", &path).unwrap();
    let text = path.to_str().unwrap();
    let memory = MemoryVfs::default();
    memory
        .insert_symlink(text, "loop", MemMeta::default())
        .unwrap();
    let spec = Spec::Local(hikasa_core::LocalPath::new(path.clone()).unwrap());
    let local = hikasa_vfs::LocalVfs::default();
    for (op, l, m) in [
        (
            "stat",
            local.stat(&spec, FollowLinks::Yes).map(|_| ()),
            memory.stat(&spec, FollowLinks::Yes).map(|_| ()),
        ),
        (
            "open",
            local.open_read(&spec).map(|_| ()),
            memory.open_read(&spec).map(|_| ()),
        ),
    ] {
        let (l, m) = (l.unwrap_err(), m.unwrap_err());
        assert_eq!(l.kind(), m.kind(), "{op}");
        assert_eq!(
            io_source(&l).map(std::io::Error::kind),
            io_source(&m).map(std::io::Error::kind),
            "{op}"
        );
        assert_eq!(l.to_string(), m.to_string(), "{op}");
    }
    // In a followed listing, the entry error is the same as well.
    let parent = Spec::Local(hikasa_core::LocalPath::new(root.clone()).unwrap());
    let opts = ListOptions {
        follow: FollowLinks::Yes,
        prefilter: None,
    };
    let l = local.list(&parent, &opts, &CancelToken::never()).unwrap();
    let m = memory.list(&parent, &opts, &CancelToken::never()).unwrap();
    assert_eq!(l.errors.len(), 1);
    assert_eq!(l.errors, m.errors);
}

/// Regression: `list` checks the token while describing entries too (every 1024 entries), not only while
/// filtering: a cancel that arrives after the filtering loop still stops the listing.
#[test]
fn list_checks_cancellation_while_describing() {
    let vfs = MemoryVfs::default();
    for i in 0..3000 {
        vfs.insert_file(&host(&format!("/d/{i:05}")), Vec::new(), MemMeta::default())
            .unwrap();
    }
    let token = CancelToken::new();
    let last = format!("{:05}", 2999);
    let cancel_at_the_end = |name: &str, _: EntryKind| {
        if name == last {
            token.cancel();
        }
        true
    };
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&cancel_at_the_end),
    };
    let r = vfs.list(&mspec("/d"), &opts, &token);
    assert!(matches!(r, Err(VfsError::Cancelled)), "{r:?}");
    // Without a cancel the listing is complete.
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&|_: &str, _: EntryKind| true),
    };
    let listing = vfs
        .list(&mspec("/d"), &opts, &CancelToken::never())
        .unwrap();
    assert_eq!(listing.len(), 3000);
}

/// §6.2.2 "unknown kinds are stat'ed": an entry whose directory record states no kind
/// (`MemoryVfs::set_kind_unknown`) is described and never passed to the prefilter, even when the prefilter would
/// decline its real kind.
#[test]
fn unknown_kinds_are_stated_and_never_prefiltered() {
    let vfs = MemoryVfs::default();
    for p in ["/r/a.txt", "/r/b.txt", "/r/sub/x"] {
        vfs.insert_file(&host(p), b"x".to_vec(), MemMeta::default())
            .unwrap();
    }
    vfs.set_kind_unknown(&host("/r/b.txt")).unwrap();
    vfs.set_kind_unknown(&host("/r/sub")).unwrap();
    assert!(matches!(
        vfs.set_kind_unknown(&host("/r/missing")),
        Err(VfsError::NotFound { .. })
    ));
    let seen = std::sync::Mutex::new(Vec::new());
    let decline_all = |name: &str, kind: EntryKind| {
        seen.lock().unwrap().push((name.to_owned(), kind));
        false
    };
    let before = vfs.stat_calls();
    let opts = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&decline_all),
    };
    let mut listing = vfs
        .list(&mspec("/r"), &opts, &CancelToken::never())
        .unwrap();
    listing.sort_by_name();
    assert_eq!(
        names(&listing),
        ["b.txt", "sub"],
        "unknown kinds are stat'ed"
    );
    assert_eq!(listing.entries[1].kind, EntryKind::Dir, "the real kind");
    assert_eq!(listing.name_only.len(), 1);
    assert_eq!(listing.name_only[0].name.as_str(), "a.txt");
    assert_eq!(vfs.stat_calls() - before, 2);
    assert_eq!(
        seen.into_inner().unwrap(),
        [("a.txt".to_owned(), EntryKind::File)],
        "the prefilter never sees unknown kinds"
    );
}
