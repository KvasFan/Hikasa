//! M2 W3 acceptance A3.1 (the mutating primitives of `Vfs`: one table test per primitive on `MemoryVfs`
//! case-sensitive and case-insensitive and on `LocalVfs` in a tempdir; the capability table of §1.3.1 per
//! back-end; the router dispatch table) and A3.2 (the new `VfsError` variants, `from_io`, `ErrorKind::NotEmpty`,
//! `VfsOp::ALL` and `Capabilities::names`). Contract M2 §1.3.1, §1.3.2, §1.3.8.

mod common;

use std::ffi::OsStr;
use std::io;
use std::sync::Arc;

use common::{Fx, ft, mspec};
use hikasa_core::spec::{PathStyle, SpecContext};
use hikasa_core::{CancelToken, CaseSensitivity, Classify, ErrorKind, FileTime, Spec};
use hikasa_vfs::{
    AttrChange, Attrs, Blob, Capabilities, DosAttrs, EntryKind, FollowLinks, LocalVfs, MemMeta,
    MemoryVfs, SetTimes, UnixMeta, Vfs, VfsError, VfsOp, VfsRouter, WriteOptions,
};

/// The outcome of a call as its error kind.
fn outcome<T>(result: &Result<T, VfsError>) -> Result<(), ErrorKind> {
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(e.kind()),
    }
}

/// The standard tree of the tables: `file`, `other` (files), `empty/`, `full/inner`, and the links `lfile` →
/// `file`, `ldir` → `full`, `broken` → `missing`. False when links could not be created (Windows without the
/// privilege): link rows are then skipped.
fn tree(fx: &Fx) -> bool {
    fx.file("file", b"content");
    fx.file("other", b"other");
    fx.dir("empty");
    fx.file("full/inner", b"inner");
    fx.link("lfile", "file", false)
        && fx.link("ldir", "full", true)
        && fx.link("broken", "missing", false)
}

fn is_link_row(rel: &str) -> bool {
    ["lfile", "ldir", "broken"]
        .iter()
        .any(|l| rel.starts_with(l))
}

// ---- A3.1 create_dir ---------------------------------------------------------------------------------------

#[test]
fn a3_1_create_dir_table() {
    use ErrorKind::*;
    // (path, parents, expected: Ok(created) or the error kind)
    let rows: &[(&str, bool, Result<bool, ErrorKind>)] = &[
        ("new", false, Ok(true)),
        ("new", true, Ok(true)),
        ("file", false, Err(AlreadyExists)),
        ("file", true, Err(NotADirectory)),
        ("empty", false, Err(AlreadyExists)),
        ("empty", true, Ok(false)),
        ("full", false, Err(AlreadyExists)),
        ("full", true, Ok(false)),
        ("lfile", false, Err(AlreadyExists)),
        ("lfile", true, Err(NotADirectory)),
        ("ldir", false, Err(AlreadyExists)),
        ("ldir", true, Ok(false)),
        ("broken", false, Err(AlreadyExists)),
        ("broken", true, Err(NotADirectory)),
        ("nope/new", false, Err(NotFound)),
        ("nope/new", true, Ok(true)),
        ("nope/deeper/new", true, Ok(true)),
        ("file/new", false, Err(NotADirectory)),
        ("file/new", true, Err(NotADirectory)),
    ];
    for &(rel, parents, expected) in rows {
        for fx in Fx::all() {
            if !tree(&fx) && is_link_row(rel) {
                continue;
            }
            let before = fx.lstat(rel).map(|e| e.kind);
            let got = fx.vfs().create_dir(&fx.spec(rel), parents);
            let label = format!("{} create_dir({rel}, parents={parents})", fx.label());
            match expected {
                Ok(created) => {
                    assert_eq!(got.as_ref().ok(), Some(&created), "{label}: {got:?}");
                    if created {
                        assert_eq!(fx.kind(rel), Some(EntryKind::Dir), "{label}");
                    } else {
                        assert_eq!(fx.lstat(rel).map(|e| e.kind), before, "{label}: unchanged");
                    }
                }
                Err(kind) => {
                    assert_eq!(outcome(&got), Err(kind), "{label}: {got:?}");
                    assert_eq!(fx.lstat(rel).map(|e| e.kind), before, "{label}: unchanged");
                }
            }
        }
    }
}

#[test]
fn a3_1_create_dir_case_variants() {
    for fx in Fx::all() {
        tree(&fx);
        // `EMPTY` is `empty` on a case-insensitive back-end.
        let got = fx.vfs().create_dir(&fx.spec("EMPTY"), true).unwrap();
        assert_eq!(got, !fx.insensitive(), "{}", fx.label());
        let got = fx.vfs().create_dir(&fx.spec("FILE"), false);
        if fx.insensitive() {
            assert_eq!(
                outcome(&got),
                Err(ErrorKind::AlreadyExists),
                "{}",
                fx.label()
            );
        } else {
            assert!(got.unwrap(), "{}", fx.label());
        }
    }
}

// ---- A3.1 remove -------------------------------------------------------------------------------------------

#[test]
fn a3_1_remove_table() {
    use ErrorKind::*;
    let rows: &[(&str, Result<(), ErrorKind>)] = &[
        ("missing", Err(NotFound)),
        ("file", Ok(())),
        ("empty", Ok(())),
        ("full", Err(NotEmpty)),
        ("lfile", Ok(())),
        ("ldir", Ok(())),
        ("broken", Ok(())),
        ("nope/x", Err(NotFound)),
        ("file/x", Err(NotADirectory)),
    ];
    for &(rel, expected) in rows {
        for fx in Fx::all() {
            if !tree(&fx) && is_link_row(rel) {
                continue;
            }
            let label = format!("{} remove({rel})", fx.label());
            let before = fx.lstat(rel).map(|e| e.kind);
            let got = fx.vfs().remove(&fx.spec(rel));
            assert_eq!(outcome(&got), expected, "{label}: {got:?}");
            if expected.is_ok() {
                assert_eq!(fx.kind(rel), None, "{label}: gone");
            } else {
                assert_eq!(fx.lstat(rel).map(|e| e.kind), before, "{label}: unchanged");
            }
            // A link's target is never touched.
            if rel != "file" {
                assert_eq!(fx.read("file"), b"content", "{label}");
            }
            assert_eq!(fx.read("full/inner"), b"inner", "{label}");
        }
    }
}

#[test]
fn a3_1_remove_root_is_refused_on_memory() {
    let vfs = MemoryVfs::default();
    vfs.insert_file(&common::host("/a"), b"x".to_vec(), MemMeta::default())
        .unwrap();
    let root = mspec("/");
    assert!(matches!(
        vfs.remove(&root),
        Err(VfsError::InvalidTarget {
            reason: "is a root folder",
            ..
        })
    ));
}

// ---- A3.1 rename -------------------------------------------------------------------------------------------

/// What a rename row checks afterwards.
#[derive(Clone, Copy)]
enum After {
    /// `to` has the kind `from` had, `from` is gone.
    Moved,
    /// Nothing changed.
    Unchanged,
}

#[test]
fn a3_1_rename_table() {
    use ErrorKind::*;
    // (from, to, replace, expected, after)
    type Row = (
        &'static str,
        &'static str,
        bool,
        Result<(), ErrorKind>,
        After,
    );
    let rows: &[Row] = &[
        ("missing", "x", false, Err(NotFound), After::Unchanged),
        ("file", "renamed", false, Ok(()), After::Moved),
        ("empty", "renamed", false, Ok(()), After::Moved),
        ("full", "renamed", false, Ok(()), After::Moved),
        ("lfile", "renamed", false, Ok(()), After::Moved),
        ("ldir", "renamed", false, Ok(()), After::Moved),
        ("broken", "renamed", false, Ok(()), After::Moved),
        ("file", "other", false, Err(AlreadyExists), After::Unchanged),
        ("file", "other", true, Ok(()), After::Moved),
        ("file", "empty", true, Err(IsADirectory), After::Unchanged),
        ("empty", "file", true, Err(NotADirectory), After::Unchanged),
        ("empty", "full", true, Err(NotEmpty), After::Unchanged),
        ("full", "empty", true, Ok(()), After::Moved),
        ("full", "empty", false, Err(AlreadyExists), After::Unchanged),
        ("other", "lfile", true, Ok(()), After::Moved),
        (
            "other",
            "lfile",
            false,
            Err(AlreadyExists),
            After::Unchanged,
        ),
        ("file", "ldir", true, Ok(()), After::Moved),
        ("empty", "ldir", true, Err(NotADirectory), After::Unchanged),
        ("lfile", "broken", true, Ok(()), After::Moved),
        ("file", "nope/x", false, Err(NotFound), After::Unchanged),
        (
            "file",
            "other/x",
            false,
            Err(NotADirectory),
            After::Unchanged,
        ),
        (
            "full",
            "full/sub",
            false,
            Err(InvalidInput),
            After::Unchanged,
        ),
    ];
    for &(from, to, replace, expected, after) in rows {
        for fx in Fx::all() {
            if !tree(&fx) && (is_link_row(from) || is_link_row(to)) {
                continue;
            }
            let label = format!("{} rename({from} -> {to}, replace={replace})", fx.label());
            let from_before = fx.lstat(from);
            let to_before = fx.lstat(to).map(|e| e.kind);
            let got = fx.vfs().rename(&fx.spec(from), &fx.spec(to), replace);
            assert_eq!(outcome(&got), expected, "{label}: {got:?}");
            match after {
                After::Moved => {
                    let from_before = from_before.unwrap();
                    assert_eq!(fx.kind(from), None, "{label}: source gone");
                    let moved = fx.lstat(to).unwrap();
                    assert_eq!(moved.kind, from_before.kind, "{label}: kind kept");
                    assert_eq!(
                        moved.link_target, from_before.link_target,
                        "{label}: link kept"
                    );
                }
                After::Unchanged => {
                    assert_eq!(
                        fx.lstat(from).map(|e| e.kind),
                        from_before.map(|e| e.kind),
                        "{label}"
                    );
                    assert_eq!(fx.lstat(to).map(|e| e.kind), to_before, "{label}");
                }
            }
            // The files links point to are never touched by a rename of (or onto) the link.
            if from != "file" && to != "file" {
                assert_eq!(fx.read("file"), b"content", "{label}");
            }
            if !["full", "empty"].contains(&from) && to != "full" {
                assert_eq!(fx.read("full/inner"), b"inner", "{label}");
            }
        }
    }
}

#[test]
fn a3_1_rename_moves_folder_contents_and_keeps_file_ids() {
    for fx in Fx::all() {
        tree(&fx);
        let id = fx.lstat("full").unwrap().file_id;
        fx.vfs()
            .rename(&fx.spec("full"), &fx.spec("empty/moved"), false)
            .unwrap();
        assert_eq!(fx.read("empty/moved/inner"), b"inner", "{}", fx.label());
        assert_eq!(
            fx.lstat("empty/moved").unwrap().file_id,
            id,
            "{}",
            fx.label()
        );
    }
}

#[test]
fn a3_1_rename_case_only() {
    for fx in Fx::all() {
        tree(&fx);
        let label = fx.label();
        fx.vfs()
            .rename(&fx.spec("file"), &fx.spec("FILE"), false)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let names = fx.names("");
        assert!(names.contains(&"FILE".to_owned()), "{label}: {names:?}");
        assert!(!names.contains(&"file".to_owned()), "{label}: {names:?}");
        assert_eq!(fx.read("FILE"), b"content", "{label}");
        // A case-only rename of a folder (same item on a case-insensitive back-end).
        fx.vfs()
            .rename(&fx.spec("full"), &fx.spec("Full"), false)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(fx.read("Full/inner"), b"inner", "{label}");
        assert!(fx.names("").contains(&"Full".to_owned()), "{label}");
        if fx.insensitive() {
            // The same item with replace = true is still a spelling change.
            fx.vfs()
                .rename(&fx.spec("Full"), &fx.spec("FULL"), true)
                .unwrap();
            assert_eq!(fx.read("FULL/inner"), b"inner", "{label}");
        }
    }
}

// ---- A3.1 set_times ----------------------------------------------------------------------------------------

#[test]
fn a3_1_set_times_table() {
    let t1 = ft(1_600_000_000);
    let modified = SetTimes {
        modified: Some(t1),
        created: None,
    };
    for fx in Fx::all() {
        let links = tree(&fx);
        let label = fx.label();
        // Links first, so the targets still have their original times.
        let mut rows = Vec::new();
        if links {
            rows.extend(["lfile", "ldir", "broken"]);
        }
        rows.extend(["file", "empty", "full"]);
        let file_mtime = fx.lstat("file").unwrap().mtime;
        let full_mtime = fx.lstat("full").unwrap().mtime;
        assert_ne!(file_mtime, Some(t1), "{label}");
        for rel in rows {
            fx.vfs()
                .set_times(&fx.spec(rel), &modified)
                .unwrap_or_else(|e| panic!("{label} set_times({rel}): {e}"));
            assert_eq!(fx.lstat(rel).unwrap().mtime, Some(t1), "{label} {rel}");
            if rel == "lfile" {
                assert_eq!(
                    fx.lstat("file").unwrap().mtime,
                    file_mtime,
                    "{label}: target kept"
                );
            }
            if rel == "ldir" {
                assert_eq!(
                    fx.lstat("full").unwrap().mtime,
                    full_mtime,
                    "{label}: target kept"
                );
            }
        }
        assert_eq!(
            outcome(&fx.vfs().set_times(&fx.spec("missing"), &modified)),
            Err(ErrorKind::NotFound),
            "{label}"
        );
        assert_eq!(
            outcome(&fx.vfs().set_times(&fx.spec("nope/x"), &modified)),
            Err(ErrorKind::NotFound),
            "{label}"
        );
        assert_eq!(
            outcome(&fx.vfs().set_times(&fx.spec("file/x"), &modified)),
            Err(ErrorKind::NotADirectory),
            "{label}"
        );
        // Nothing to change: success, nothing changed.
        let before = fx.lstat("other").unwrap();
        fx.vfs()
            .set_times(&fx.spec("other"), &SetTimes::default())
            .unwrap();
        assert_eq!(fx.lstat("other").unwrap().mtime, before.mtime, "{label}");
    }
}

#[test]
fn a3_1_set_times_created_memory() {
    let (t1, t2) = (ft(1_600_000_000), ft(1_500_000_000));
    let both = SetTimes {
        modified: Some(t1),
        created: Some(t2),
    };
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let fx = Fx::mem(case);
        tree(&fx);
        fx.vfs().set_times(&fx.spec("file"), &both).unwrap();
        let entry = fx.lstat("file").unwrap();
        assert_eq!((entry.mtime, entry.ctime), (Some(t1), Some(t2)));
        // Created only.
        fx.vfs()
            .set_times(
                &fx.spec("other"),
                &SetTimes {
                    modified: None,
                    created: Some(t2),
                },
            )
            .unwrap();
        assert_eq!(fx.lstat("other").unwrap().ctime, Some(t2));
        assert_eq!(fx.lstat("other").unwrap().mtime, None);
        // Without SET_CREATED: OperationUnsupported, nothing changed.
        fx.memory()
            .set_capabilities(Capabilities::all().difference(Capabilities::SET_CREATED));
        let before = fx.memory().dump();
        let err = fx.vfs().set_times(&fx.spec("empty"), &both).unwrap_err();
        assert!(
            matches!(
                err,
                VfsError::OperationUnsupported {
                    op: VfsOp::SetTimes,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(fx.memory().dump(), before);
        // Modified only still works.
        fx.vfs()
            .set_times(
                &fx.spec("empty"),
                &SetTimes {
                    modified: Some(t1),
                    created: None,
                },
            )
            .unwrap();
    }
}

#[test]
fn a3_1_set_times_created_local() {
    let fx = Fx::local();
    tree(&fx);
    let (t1, t2) = (ft(1_600_000_000), ft(1_500_000_000));
    let both = SetTimes {
        modified: Some(t1),
        created: Some(t2),
    };
    let caps = fx.vfs().capabilities(&fx.spec("")).unwrap();
    let before = fx.lstat("file").unwrap();
    let got = fx.vfs().set_times(&fx.spec("file"), &both);
    if cfg!(any(target_os = "macos", windows)) {
        assert!(caps.contains(Capabilities::SET_CREATED));
        got.unwrap();
        let entry = fx.lstat("file").unwrap();
        assert_eq!(entry.mtime, Some(t1));
        assert_eq!(entry.ctime, Some(t2));
    } else {
        // Linux and the other Unix systems: no SET_CREATED → OperationUnsupported, nothing changed.
        assert!(!caps.contains(Capabilities::SET_CREATED));
        assert!(
            matches!(
                got,
                Err(VfsError::OperationUnsupported {
                    op: VfsOp::SetTimes,
                    ..
                })
            ),
            "{got:?}"
        );
        assert_eq!(fx.lstat("file").unwrap().mtime, before.mtime);
    }
}

// ---- A3.1 set_attrs ----------------------------------------------------------------------------------------

fn mode_of(fx: &Fx, rel: &str) -> Option<u32> {
    match fx.lstat(rel)?.attrs {
        Attrs::Unix { meta } => Some(meta.mode),
        _ => None,
    }
}

#[test]
fn a3_1_set_attrs_mode_table() {
    let mut fixtures = vec![
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Insensitive),
    ];
    if cfg!(unix) {
        fixtures.push(Fx::local());
    }
    for fx in fixtures {
        let links = tree(&fx);
        let label = fx.label();
        for (rel, mode) in [
            ("file", 0o640),
            ("empty", 0o750),
            ("full", 0o700),
            ("other", 0o4755),
        ] {
            fx.vfs()
                .set_attrs(&fx.spec(rel), &AttrChange::Mode(mode))
                .unwrap_or_else(|e| panic!("{label} {rel}: {e}"));
            assert_eq!(mode_of(&fx, rel), Some(mode), "{label} {rel}");
        }
        if links {
            for rel in ["lfile", "ldir", "broken"] {
                let err = fx
                    .vfs()
                    .set_attrs(&fx.spec(rel), &AttrChange::Mode(0o600))
                    .unwrap_err();
                assert!(
                    matches!(
                        err,
                        VfsError::OperationUnsupported {
                            op: VfsOp::SetAttrs,
                            ..
                        }
                    ),
                    "{label} {rel}: {err:?}"
                );
            }
            assert_eq!(
                mode_of(&fx, "file"),
                Some(0o640),
                "{label}: link target kept"
            );
        }
        let err = fx
            .vfs()
            .set_attrs(&fx.spec("file"), &AttrChange::Mode(0o170_644))
            .unwrap_err();
        assert!(
            matches!(err, VfsError::InvalidAttributes { .. }),
            "{label}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            outcome(
                &fx.vfs()
                    .set_attrs(&fx.spec("missing"), &AttrChange::Mode(0o600))
            ),
            Err(ErrorKind::NotFound),
            "{label}"
        );
        // Overlapping DOS set and clear: invalid on every back-end.
        let overlap = AttrChange::Dos {
            set: DosAttrs::READONLY,
            clear: DosAttrs::READONLY,
        };
        assert!(
            matches!(
                fx.vfs().set_attrs(&fx.spec("file"), &overlap),
                Err(VfsError::InvalidAttributes { .. })
            ),
            "{label}"
        );
    }
}

#[test]
fn a3_1_set_attrs_wrong_family() {
    let dos_r = AttrChange::Dos {
        set: DosAttrs::READONLY,
        clear: DosAttrs::empty(),
    };
    // MemoryVfs: the family of the item's attributes decides.
    let vfs = MemoryVfs::default();
    let unix = Attrs::Unix {
        meta: UnixMeta {
            mode: 0o644,
            uid: 7,
            gid: 8,
        },
    };
    let dos = Attrs::Dos {
        attrs: DosAttrs::HIDDEN,
    };
    let meta = |attrs| MemMeta {
        attrs: Some(attrs),
        ..MemMeta::default()
    };
    vfs.insert_file(&common::host("/u"), b"u".to_vec(), meta(unix))
        .unwrap();
    vfs.insert_file(&common::host("/d"), b"d".to_vec(), meta(dos))
        .unwrap();
    let unsupported = |r: Result<(), VfsError>| {
        matches!(
            r,
            Err(VfsError::OperationUnsupported {
                op: VfsOp::SetAttrs,
                ..
            })
        )
    };
    assert!(unsupported(vfs.set_attrs(&mspec("/u"), &dos_r)));
    assert!(unsupported(
        vfs.set_attrs(&mspec("/d"), &AttrChange::Mode(0o600))
    ));
    // Within the family: applied, owner kept (D-99).
    vfs.set_attrs(&mspec("/u"), &AttrChange::Mode(0o600))
        .unwrap();
    assert_eq!(
        vfs.stat(&mspec("/u"), FollowLinks::No).unwrap().attrs,
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o600,
                uid: 7,
                gid: 8
            }
        }
    );
    let dos_change = AttrChange::Dos {
        set: DosAttrs::READONLY,
        clear: DosAttrs::HIDDEN,
    };
    vfs.set_attrs(&mspec("/d"), &dos_change).unwrap();
    assert_eq!(
        vfs.stat(&mspec("/d"), FollowLinks::No).unwrap().attrs,
        Attrs::Dos {
            attrs: DosAttrs::READONLY
        }
    );
    // Without attributes the back-end's family capability decides.
    vfs.insert_file(&common::host("/n"), b"n".to_vec(), MemMeta::default())
        .unwrap();
    vfs.set_capabilities(Capabilities::all().difference(Capabilities::DOS_ATTRS));
    assert!(unsupported(vfs.set_attrs(&mspec("/n"), &dos_r)));
    vfs.set_capabilities(Capabilities::all().difference(Capabilities::UNIX_MODE));
    assert!(unsupported(
        vfs.set_attrs(&mspec("/n"), &AttrChange::Mode(0o600))
    ));
    vfs.set_attrs(&mspec("/n"), &dos_r).unwrap();

    // LocalVfs: the platform's family.
    let fx = Fx::local();
    tree(&fx);
    #[cfg(unix)]
    assert!(unsupported(fx.vfs().set_attrs(&fx.spec("file"), &dos_r)));
    #[cfg(windows)]
    assert!(unsupported(
        fx.vfs()
            .set_attrs(&fx.spec("file"), &AttrChange::Mode(0o600))
    ));
}

#[cfg(windows)]
#[test]
fn a3_1_set_attrs_dos_windows() {
    let fx = Fx::local();
    tree(&fx);
    let r = |set, clear| AttrChange::Dos { set, clear };
    fx.vfs()
        .set_attrs(&fx.spec("file"), &r(DosAttrs::READONLY, DosAttrs::empty()))
        .unwrap();
    assert!(
        std::fs::metadata(fx.path("file"))
            .unwrap()
            .permissions()
            .readonly()
    );
    fx.vfs()
        .set_attrs(&fx.spec("file"), &r(DosAttrs::empty(), DosAttrs::READONLY))
        .unwrap();
    assert!(
        !std::fs::metadata(fx.path("file"))
            .unwrap()
            .permissions()
            .readonly()
    );
    let err = fx
        .vfs()
        .set_attrs(&fx.spec("file"), &r(DosAttrs::HIDDEN, DosAttrs::empty()))
        .unwrap_err();
    assert!(
        matches!(
            err,
            VfsError::Unsupported {
                what: "changing DOS attributes other than R",
                ..
            }
        ),
        "{err:?}"
    );
}

// ---- A3.1 create_symlink -----------------------------------------------------------------------------------

#[test]
fn a3_1_create_symlink_table() {
    use ErrorKind::*;
    let rows: &[(&str, &str, Result<(), ErrorKind>)] = &[
        ("nl", "file", Ok(())),
        ("nl", "does/not/../exist", Ok(())),
        ("nl", "missing", Ok(())),
        ("empty/nl", "../file", Ok(())),
        ("file", "x", Err(AlreadyExists)),
        ("empty", "x", Err(AlreadyExists)),
        ("broken", "x", Err(AlreadyExists)),
        ("lfile", "x", Err(AlreadyExists)),
        ("nope/nl", "x", Err(NotFound)),
        ("file/nl", "x", Err(NotADirectory)),
    ];
    for &(rel, target, expected) in rows {
        for fx in Fx::all() {
            if !tree(&fx) {
                continue;
            }
            let label = format!("{} create_symlink({rel} -> {target})", fx.label());
            let before = fx.lstat(rel).map(|e| (e.kind, e.link_target));
            let got = fx
                .vfs()
                .create_symlink(&fx.spec(rel), OsStr::new(target), false);
            if matches!(&got, Err(VfsError::PermissionDenied { .. })) && cfg!(windows) {
                eprintln!("{label}: skipped (no privilege to create links)");
                continue;
            }
            assert_eq!(outcome(&got), expected, "{label}: {got:?}");
            if expected.is_ok() {
                let entry = fx.lstat(rel).unwrap();
                assert_eq!(entry.kind, EntryKind::Symlink, "{label}");
                // Stored verbatim (on Windows the OS may normalise separators).
                if cfg!(unix) || fx.is_mem() {
                    assert_eq!(entry.link_target.as_deref(), Some(target), "{label}");
                }
            } else {
                assert_eq!(
                    fx.lstat(rel).map(|e| (e.kind, e.link_target)),
                    before,
                    "{label}"
                );
            }
        }
    }
    // A link created by the primitive resolves like any other.
    for fx in Fx::all() {
        if !tree(&fx) {
            continue;
        }
        if fx
            .vfs()
            .create_symlink(&fx.spec("empty/nl"), OsStr::new("../file"), false)
            .is_ok()
        {
            assert_eq!(fx.read("empty/nl"), b"content", "{}", fx.label());
        }
    }
}

// ---- A3.1 move_to_trash ------------------------------------------------------------------------------------

#[test]
fn a3_1_move_to_trash_table_memory() {
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let fx = Fx::mem(case);
        tree(&fx);
        let vfs = fx.memory();
        assert_eq!(
            outcome(&vfs.move_to_trash(&fx.spec("missing"))),
            Err(ErrorKind::NotFound)
        );
        for rel in ["file", "lfile", "ldir", "broken", "full", "empty"] {
            vfs.move_to_trash(&fx.spec(rel)).unwrap();
            assert_eq!(fx.kind(rel), None, "{rel}");
        }
        assert_eq!(
            vfs.trashed(),
            ["file", "lfile", "ldir", "broken", "full", "empty"]
                .map(|r| fx.text(r))
                .to_vec()
        );
        assert!(matches!(
            vfs.move_to_trash(&mspec("/")),
            Err(VfsError::InvalidTarget {
                reason: "is a root folder",
                ..
            })
        ));
    }
}

#[test]
fn a3_1_move_to_trash_local_refusals_never_reach_the_trash() {
    // Only the refusals that are decided before the recycle bin is called (the real trash is tested in a
    // subprocess with an isolated home, tests/trash.rs).
    let fx = Fx::local();
    let got = fx.vfs().move_to_trash(&fx.spec("missing"));
    assert_eq!(outcome(&got), Err(ErrorKind::NotFound), "{got:?}");
    let root = Spec::Local(
        hikasa_core::LocalPath::new(std::path::PathBuf::from(common::host("/"))).unwrap(),
    );
    assert!(matches!(
        fx.vfs().move_to_trash(&root),
        Err(VfsError::InvalidTarget {
            reason: "is a root folder",
            ..
        })
    ));
}

// ---- A3.1 copy_file_fast -----------------------------------------------------------------------------------

#[test]
fn a3_1_copy_file_fast_table() {
    let cancel = CancelToken::never();
    let opts = WriteOptions::default();
    // MemoryVfs has no fast path.
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    tree(&fx);
    assert_eq!(
        fx.vfs()
            .copy_file_fast(&fx.spec("file"), &fx.spec("copy"), &opts, &cancel)
            .unwrap(),
        None
    );
    // LocalVfs: a regular file up to FAST_COPY_MAX.
    let fx = Fx::local();
    tree(&fx);
    let got = fx
        .vfs()
        .copy_file_fast(&fx.spec("file"), &fx.spec("copy"), &opts, &cancel)
        .unwrap();
    assert_eq!(got, Some(7));
    assert_eq!(fx.read("copy"), b"content");
    // A folder source: no fast path.
    assert_eq!(
        fx.vfs()
            .copy_file_fast(&fx.spec("full"), &fx.spec("copy2"), &opts, &cancel)
            .unwrap(),
        None
    );
    // An existing target without overwrite: AlreadyExists, untouched; with overwrite: replaced.
    let got = fx
        .vfs()
        .copy_file_fast(&fx.spec("other"), &fx.spec("copy"), &opts, &cancel);
    assert_eq!(outcome(&got), Err(ErrorKind::AlreadyExists));
    assert_eq!(fx.read("copy"), b"content");
    let overwrite = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    fx.vfs()
        .copy_file_fast(&fx.spec("other"), &fx.spec("copy"), &overwrite, &cancel)
        .unwrap();
    assert_eq!(fx.read("copy"), b"other");
    // A cancelled token: nothing written.
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let got = fx
        .vfs()
        .copy_file_fast(&fx.spec("file"), &fx.spec("copy3"), &opts, &cancelled);
    assert!(matches!(got, Err(VfsError::Cancelled)), "{got:?}");
    assert_eq!(fx.kind("copy3"), None);
    assert_eq!(
        fx.names("").iter().filter(|n| n.starts_with('.')).count(),
        0,
        "no temporary left"
    );
}

// ---- A3.1 capabilities -------------------------------------------------------------------------------------

const W3_BITS: Capabilities = Capabilities::SET_TIMES
    .union(Capabilities::MKDIR)
    .union(Capabilities::REMOVE)
    .union(Capabilities::RENAME)
    .union(Capabilities::SET_ATTRS)
    .union(Capabilities::TRASH)
    .union(Capabilities::CREATE_SYMLINKS)
    .union(Capabilities::SET_CREATED);

const BLOB_CAPS: Capabilities = Capabilities::READ
    .union(Capabilities::SEEK)
    .union(Capabilities::MTIME);

fn blob() -> Blob {
    Blob {
        bytes: Arc::from(&b"blob"[..]),
        mtime: FileTime::from_unix(1_700_000_000, 0).unwrap(),
        name: None,
    }
}

#[test]
fn a3_1_capability_table_per_back_end() {
    // MemoryVfs: all 21 (case-sensitive only when it is).
    let sensitive = MemoryVfs::new(CaseSensitivity::Sensitive);
    sensitive
        .insert_dir(&common::host("/w"), MemMeta::default())
        .unwrap();
    // `LOCAL_PATH` stays off: `MemoryVfs` has no local paths (nothing is ever mapped), as in M1; the other 20
    // bits, all seven W3 bits and `SET_TIMES` among them, are on.
    let memory_all = Capabilities::all().difference(Capabilities::LOCAL_PATH);
    assert_eq!(sensitive.capabilities(&mspec("/w")).unwrap(), memory_all);
    assert!(
        sensitive
            .capabilities(&mspec("/w"))
            .unwrap()
            .contains(W3_BITS)
    );
    let insensitive = MemoryVfs::new(CaseSensitivity::Insensitive);
    assert_eq!(
        insensitive.capabilities(&mspec("/w")).unwrap(),
        memory_all.difference(Capabilities::CASE_SENSITIVE)
    );

    // LocalVfs: the platform's W3 set.
    let dir = tempfile::tempdir().unwrap();
    let local = LocalVfs::default();
    let caps = local.capabilities(&common::lspec(dir.path())).unwrap();
    let unix_set = Capabilities::SET_TIMES
        | Capabilities::MKDIR
        | Capabilities::REMOVE
        | Capabilities::RENAME
        | Capabilities::SET_ATTRS
        | Capabilities::TRASH
        | Capabilities::CREATE_SYMLINKS;
    let expected = if cfg!(any(target_os = "macos", windows)) {
        unix_set | Capabilities::SET_CREATED
    } else {
        unix_set
    };
    assert_eq!(caps.intersection(W3_BITS), expected, "{:?}", caps.names());
    if cfg!(unix) {
        assert!(caps.contains(Capabilities::UNIX_MODE));
        assert!(!caps.contains(Capabilities::DOS_ATTRS));
    }
    if cfg!(windows) {
        assert!(caps.contains(Capabilities::DOS_ATTRS));
    }

    // Blobs: unchanged, no W3 bit.
    let router = VfsRouter::new(LocalVfs::default())
        .with_stdin(blob())
        .with_clipboard(blob());
    for spec in [Spec::Stdin, Spec::Clipboard] {
        assert_eq!(router.capabilities(&spec).unwrap(), BLOB_CAPS);
    }
    let bare = VfsRouter::new(LocalVfs::default());
    for spec in [Spec::Stdin, Spec::Clipboard] {
        assert_eq!(bare.capabilities(&spec).unwrap(), BLOB_CAPS);
    }
}

#[test]
fn a3_1_memory_removed_capability_makes_the_primitive_unsupported() {
    type Call = fn(&MemoryVfs) -> Result<(), VfsError>;
    let rows: [(Capabilities, VfsOp, Call); 7] = [
        (Capabilities::MKDIR, VfsOp::Mkdir, |v| {
            v.create_dir(&mspec("/w/new"), false).map(|_| ())
        }),
        (Capabilities::REMOVE, VfsOp::Remove, |v| {
            v.remove(&mspec("/w/file"))
        }),
        (Capabilities::RENAME, VfsOp::Rename, |v| {
            v.rename(&mspec("/w/file"), &mspec("/w/b"), false)
        }),
        (Capabilities::SET_TIMES, VfsOp::SetTimes, |v| {
            v.set_times(
                &mspec("/w/file"),
                &SetTimes {
                    modified: Some(ft(1)),
                    created: None,
                },
            )
        }),
        (Capabilities::SET_ATTRS, VfsOp::SetAttrs, |v| {
            v.set_attrs(&mspec("/w/file"), &AttrChange::Mode(0o600))
        }),
        (Capabilities::CREATE_SYMLINKS, VfsOp::Symlink, |v| {
            v.create_symlink(&mspec("/w/l"), OsStr::new("file"), false)
        }),
        (Capabilities::TRASH, VfsOp::Trash, |v| {
            v.move_to_trash(&mspec("/w/file"))
        }),
    ];
    for (cap, op, call) in rows {
        let vfs = MemoryVfs::default();
        vfs.insert_file(&common::host("/w/file"), b"x".to_vec(), MemMeta::default())
            .unwrap();
        // With the bit the call succeeds.
        call(&vfs).unwrap_or_else(|e| panic!("{op:?} with the capability: {e}"));
        let vfs = MemoryVfs::default();
        vfs.insert_file(&common::host("/w/file"), b"x".to_vec(), MemMeta::default())
            .unwrap();
        vfs.set_capabilities(Capabilities::all().difference(cap));
        let before = vfs.dump();
        let err = call(&vfs).unwrap_err();
        assert!(
            matches!(&err, VfsError::OperationUnsupported { op: o, .. } if *o == op),
            "{op:?}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        assert_eq!(vfs.dump(), before, "{op:?}: nothing changed");
    }
}

// ---- A3.1 router dispatch ----------------------------------------------------------------------------------

fn parse(text: &str) -> Spec {
    let ctx = SpecContext {
        style: PathStyle::HOST,
        cwd: None,
        home: None,
    };
    Spec::parse(text, &ctx).unwrap()
}

/// Every single-spec primitive through `vfs` at `spec`, labelled.
fn primitives(vfs: &dyn Vfs, spec: &Spec) -> Vec<(VfsOp, Result<(), VfsError>)> {
    let times = SetTimes {
        modified: Some(ft(1_600_000_000)),
        created: None,
    };
    vec![
        (VfsOp::Mkdir, vfs.create_dir(spec, true).map(|_| ())),
        (VfsOp::SetTimes, vfs.set_times(spec, &times)),
        (
            VfsOp::SetAttrs,
            vfs.set_attrs(spec, &AttrChange::Mode(0o644)),
        ),
        (
            VfsOp::Symlink,
            vfs.create_symlink(spec, OsStr::new("x"), false),
        ),
        (VfsOp::Trash, vfs.move_to_trash(spec)),
        (VfsOp::Remove, vfs.remove(spec)),
    ]
}

#[test]
fn a3_1_router_dispatch_local() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let router = VfsRouter::new(LocalVfs::default());
    let spec = |name: &str| common::lspec_new(&base.join(name));
    assert!(router.create_dir(&spec("d"), false).unwrap());
    std::fs::write(base.join("f"), b"abc").unwrap();
    router
        .set_times(
            &spec("f"),
            &SetTimes {
                modified: Some(ft(1_600_000_000)),
                created: None,
            },
        )
        .unwrap();
    assert_eq!(
        router.stat(&spec("f"), FollowLinks::No).unwrap().mtime,
        Some(ft(1_600_000_000))
    );
    #[cfg(unix)]
    router
        .set_attrs(&spec("f"), &AttrChange::Mode(0o600))
        .unwrap();
    router.rename(&spec("f"), &spec("g"), false).unwrap();
    assert!(base.join("g").exists());
    #[cfg(unix)]
    {
        router
            .create_symlink(&spec("l"), OsStr::new("g"), false)
            .unwrap();
        router.remove(&spec("l")).unwrap();
    }
    let cancel = CancelToken::never();
    assert_eq!(
        router
            .copy_file_fast(&spec("g"), &spec("h"), &WriteOptions::default(), &cancel)
            .unwrap(),
        Some(3)
    );
    router.remove(&spec("h")).unwrap();
    router.remove(&spec("d")).unwrap();
    assert!(!base.join("d").exists());
}

#[test]
fn a3_1_router_dispatch_blobs_unsupported_schemes_and_mixed_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let local = common::lspec(dir.path());
    std::fs::write(dir.path().join("f"), b"x").unwrap();
    let file = common::lspec(&dir.path().join("f"));
    let cancel = CancelToken::never();
    let opts = WriteOptions::default();
    let with_blobs = VfsRouter::new(LocalVfs::default())
        .with_stdin(blob())
        .with_clipboard(blob());
    let bare = VfsRouter::new(LocalVfs::default());
    for router in [&with_blobs, &bare] {
        // Blobs: OperationUnsupported for every primitive.
        for spec in [Spec::Stdin, Spec::Clipboard] {
            for (op, got) in primitives(router, &spec) {
                assert!(
                    matches!(&got, Err(VfsError::OperationUnsupported { op: o, .. }) if *o == op),
                    "{spec:?} {op:?}: {got:?}"
                );
            }
            let got = router.rename(&spec, &spec, false);
            assert!(
                matches!(
                    got,
                    Err(VfsError::OperationUnsupported {
                        op: VfsOp::Rename,
                        ..
                    })
                ),
                "{got:?}"
            );
            assert_eq!(
                router.copy_file_fast(&spec, &file, &opts, &cancel).unwrap(),
                None
            );
        }
        // An unsupported scheme: Unsupported "<scheme> specs" (D-05).
        for (text, what) in [
            ("ftp://host/x", "ftp specs"),
            ("sftp://host/x", "sftp specs"),
            ("https://host/x", "https specs"),
        ] {
            let spec = parse(text);
            for (op, got) in primitives(router, &spec) {
                assert!(
                    matches!(&got, Err(VfsError::Unsupported { what: w, .. }) if *w == what),
                    "{text} {op:?}: {got:?}"
                );
            }
            for got in [
                router.rename(&spec, &file, false),
                router.rename(&file, &spec, false),
                router.rename(&spec, &spec, false),
            ] {
                assert!(
                    matches!(&got, Err(VfsError::Unsupported { what: w, .. }) if *w == what),
                    "{text} rename: {got:?}"
                );
            }
            assert_eq!(
                router.copy_file_fast(&spec, &file, &opts, &cancel).unwrap(),
                None
            );
            assert_eq!(
                router.copy_file_fast(&file, &spec, &opts, &cancel).unwrap(),
                None
            );
        }
        // Mixed kinds: rename is OperationUnsupported, copy_file_fast has no fast path.
        for (from, to) in [
            (&file, &Spec::Stdin),
            (&Spec::Clipboard, &local),
            (&Spec::Stdin, &Spec::Clipboard),
        ] {
            let got = router.rename(from, to, false);
            assert!(
                matches!(
                    got,
                    Err(VfsError::OperationUnsupported {
                        op: VfsOp::Rename,
                        ..
                    })
                ),
                "{from:?} -> {to:?}: {got:?}"
            );
            assert_eq!(
                router.copy_file_fast(from, to, &opts, &cancel).unwrap(),
                None
            );
        }
    }
    assert!(dir.path().join("f").exists(), "nothing local was touched");
}

#[test]
fn a3_1_default_trait_bodies_are_unsupported() {
    /// A read-only back-end that implements only the M1 methods.
    struct ReadOnly;
    impl Vfs for ReadOnly {
        fn capabilities(&self, _at: &Spec) -> Result<Capabilities, VfsError> {
            Ok(Capabilities::READ)
        }
        fn stat(&self, spec: &Spec, _f: FollowLinks) -> Result<hikasa_vfs::Entry, VfsError> {
            Err(VfsError::NotFound {
                spec: spec.canonical(),
            })
        }
        fn list(
            &self,
            dir: &Spec,
            _o: &hikasa_vfs::ListOptions<'_>,
            _c: &CancelToken,
        ) -> Result<hikasa_vfs::Listing, VfsError> {
            Err(VfsError::NotFound {
                spec: dir.canonical(),
            })
        }
        fn open_read(&self, spec: &Spec) -> Result<Box<dyn hikasa_vfs::ReadSeek>, VfsError> {
            Err(VfsError::NotFound {
                spec: spec.canonical(),
            })
        }
        fn open_write(
            &self,
            spec: &Spec,
            _o: &WriteOptions,
        ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, VfsError> {
            Err(VfsError::NotFound {
                spec: spec.canonical(),
            })
        }
        fn child(&self, dir: &Spec, _n: &hikasa_vfs::FileName) -> Result<Spec, VfsError> {
            Ok(dir.clone())
        }
    }
    let spec = mspec("/x");
    for (op, got) in primitives(&ReadOnly, &spec) {
        assert!(
            matches!(&got, Err(VfsError::OperationUnsupported { op: o, .. }) if *o == op),
            "{op:?}: {got:?}"
        );
    }
    assert!(matches!(
        ReadOnly.rename(&spec, &spec, true),
        Err(VfsError::OperationUnsupported {
            op: VfsOp::Rename,
            ..
        })
    ));
    assert_eq!(
        ReadOnly
            .copy_file_fast(
                &spec,
                &spec,
                &WriteOptions::default(),
                &CancelToken::never()
            )
            .unwrap(),
        None
    );
}

// ---- A3.2 errors -------------------------------------------------------------------------------------------

#[test]
fn a3_2_new_error_variants() {
    // (error, kind, message, spec(), io kind)
    let rows: Vec<(VfsError, ErrorKind, &str, Option<&str>, io::ErrorKind)> = vec![
        (
            VfsError::NotEmpty { spec: "/d".into() },
            ErrorKind::NotEmpty,
            "/d: folder is not empty",
            Some("/d"),
            io::ErrorKind::DirectoryNotEmpty,
        ),
        (
            VfsError::CrossDevice {
                from: "/a".into(),
                to: "/b".into(),
            },
            ErrorKind::Unsupported,
            "/a -> /b: cannot rename across file systems",
            Some("/a"),
            io::ErrorKind::CrossesDevices,
        ),
        (
            VfsError::OperationUnsupported {
                spec: "-".into(),
                op: VfsOp::SetTimes,
            },
            ErrorKind::Unsupported,
            "-: set_times is not supported here",
            Some("-"),
            io::ErrorKind::Unsupported,
        ),
        (
            VfsError::InvalidTarget {
                spec: "/t".into(),
                reason: "is inside the source folder",
            },
            ErrorKind::InvalidInput,
            "/t: is inside the source folder",
            Some("/t"),
            io::ErrorKind::InvalidInput,
        ),
        (
            VfsError::Unavailable {
                spec: "clipboard://".into(),
                reason: "no clipboard".into(),
            },
            ErrorKind::Unsupported,
            "clipboard://: no clipboard",
            Some("clipboard://"),
            io::ErrorKind::Unsupported,
        ),
    ];
    for (error, kind, message, spec, io_kind) in rows {
        assert_eq!(error.kind(), kind, "{error:?}");
        assert_eq!(error.to_string(), message);
        assert_eq!(error.spec(), spec, "{error:?}");
        let debug = format!("{error:?}");
        let io_error = io::Error::from(error);
        assert_eq!(io_error.kind(), io_kind, "{debug}");
        // Every variant classifies the same after the conversion (`CrossesDevices` → `unsupported`, W3 note).
        assert_eq!(hikasa_core::io_error_kind(&io_error), kind, "{debug}");
        // from_io recovers the original.
        let back = VfsError::from_io("ignored", VfsOp::Stat, io_error);
        assert_eq!(format!("{back:?}"), debug);
    }
    // Every reason text of InvalidTarget.
    for reason in [
        "is the source itself",
        "is inside the source folder",
        "is not an existing folder",
        "needs a file name",
        "is a root folder",
        "changed since the plan was made",
    ] {
        let e = VfsError::InvalidTarget {
            spec: "/x".into(),
            reason,
        };
        assert_eq!(e.to_string(), format!("/x: {reason}"));
    }
    // Every VfsOp in the OperationUnsupported message.
    for op in VfsOp::ALL {
        let e = VfsError::OperationUnsupported {
            spec: "/x".into(),
            op,
        };
        assert_eq!(
            e.to_string(),
            format!("/x: {} is not supported here", op.as_str())
        );
    }
}

#[test]
fn a3_2_from_io_new_kinds() {
    let e = VfsError::from_io(
        "/d",
        VfsOp::Remove,
        io::Error::from(io::ErrorKind::DirectoryNotEmpty),
    );
    assert!(
        matches!(&e, VfsError::NotEmpty { spec } if spec == "/d"),
        "{e:?}"
    );
    let e = VfsError::from_io(
        "/a",
        VfsOp::Rename,
        io::Error::from(io::ErrorKind::CrossesDevices),
    );
    assert!(
        matches!(&e, VfsError::CrossDevice { from, to } if from == "/a" && to == "/a"),
        "{e:?}"
    );
    assert_eq!(
        hikasa_core::io_error_kind(&io::Error::from(io::ErrorKind::DirectoryNotEmpty)),
        ErrorKind::NotEmpty
    );
}

#[test]
fn a3_2_error_kind_not_empty_serde() {
    assert_eq!(ErrorKind::NotEmpty.as_str(), "not_empty");
    assert_eq!(
        common::json::to_value(&ErrorKind::NotEmpty),
        common::json::J::Str("not_empty".into())
    );
    let back: ErrorKind =
        common::json::from_value(common::json::J::Str("not_empty".into())).unwrap();
    assert_eq!(back, ErrorKind::NotEmpty);
}

#[test]
fn a3_2_vfs_op_all_and_capability_names() {
    let names: Vec<&str> = VfsOp::ALL.iter().map(|op| op.as_str()).collect();
    assert_eq!(
        names,
        [
            "stat",
            "list",
            "open",
            "read",
            "write",
            "commit",
            "map",
            "mkdir",
            "remove",
            "rename",
            "set_times",
            "set_attrs",
            "symlink",
            "trash"
        ]
    );
    for op in VfsOp::ALL {
        assert_eq!(
            common::json::to_value(&op),
            common::json::J::Str(op.as_str().into())
        );
        let back: VfsOp =
            common::json::from_value(common::json::J::Str(op.as_str().into())).unwrap();
        assert_eq!(back, op);
    }
    assert_eq!(
        Capabilities::all().names(),
        [
            "read",
            "write",
            "list",
            "seek",
            "mtime",
            "ctime",
            "set_times",
            "dos_attrs",
            "unix_mode",
            "unix_owner",
            "symlinks",
            "crc_provided",
            "local_path",
            "case_sensitive",
            "mkdir",
            "remove",
            "rename",
            "set_attrs",
            "trash",
            "create_symlinks",
            "set_created"
        ]
    );
    for (bit, name) in Capabilities::all().names().iter().enumerate() {
        assert_eq!(
            Capabilities::parse_name(name).unwrap().bits(),
            1 << bit,
            "{name}"
        );
    }
}

// ---- W3 review fixes (§1.3.12) -----------------------------------------------------------------------------

#[test]
fn fix_open_write_through_a_dangling_link_creates_the_file_it_names() {
    // M1 save semantics on every back-end: the link stays, the file it names is created (`replace_link`
    // replaces the link itself).
    for fx in Fx::all() {
        let label = fx.label();
        fx.dir("d");
        if !(fx.link("broken", "d/missing", false) && fx.link("chain", "broken", false)) {
            continue;
        }
        for (link, content) in [("broken", &b"one"[..]), ("chain", &b"two"[..])] {
            let opts = WriteOptions {
                overwrite: true,
                ..WriteOptions::default()
            };
            let mut w = fx.vfs().open_write(&fx.spec(link), &opts).unwrap();
            io::Write::write_all(&mut w, content).unwrap();
            w.commit().unwrap();
            assert_eq!(fx.kind(link), Some(EntryKind::Symlink), "{label} {link}");
            assert_eq!(fx.read("d/missing"), content, "{label} {link}");
            fx.vfs().remove(&fx.spec("d/missing")).unwrap();
        }
        // A link whose named folder is missing: `NotFound`, nothing created.
        assert!(fx.link("nofolder", "gone/x", false));
        let opts = WriteOptions {
            overwrite: true,
            ..WriteOptions::default()
        };
        let result = fx
            .vfs()
            .open_write(&fx.spec("nofolder"), &opts)
            .and_then(|w| w.commit());
        assert_eq!(outcome(&result), Err(ErrorKind::NotFound), "{label}");
        assert!(fx.lstat("gone").is_none(), "{label}");
        // With `replace_link` the dangling link itself is replaced by the file.
        let opts = WriteOptions {
            overwrite: true,
            replace_link: true,
            ..WriteOptions::default()
        };
        let mut w = fx.vfs().open_write(&fx.spec("broken"), &opts).unwrap();
        io::Write::write_all(&mut w, b"file").unwrap();
        w.commit().unwrap();
        assert_eq!(fx.kind("broken"), Some(EntryKind::File), "{label}");
        assert!(fx.lstat("d/missing").is_none(), "{label}");
    }
}

#[test]
fn fix_primitives_refuse_dot_dot_items() {
    for fx in Fx::all() {
        let label = fx.label();
        fx.file("d/x", b"x");
        let up = fx.spec("d/..");
        let vfs = fx.vfs();
        let refused = |result: Result<(), VfsError>, what: &str| {
            let error = result.expect_err(what);
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidInput,
                "{label} {what}: {error}"
            );
        };
        if fx.is_mem() {
            // `MemoryVfs` resolves `..` like the OS; the ops layer refuses such items (`ops` tests).
            continue;
        }
        refused(vfs.remove(&up), "remove");
        refused(vfs.move_to_trash(&up), "trash");
        refused(vfs.rename(&up, &fx.spec("e"), false), "rename from");
        refused(
            vfs.rename(&fx.spec("d/x"), &fx.spec("d/x/.."), true),
            "rename to",
        );
        assert_eq!(fx.read("d/x"), b"x", "{label}");
        assert!(fx.lstat("e").is_none(), "{label}");
    }
}
