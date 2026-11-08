//! `Vfs::writable` (W10a, §1.10.7, A10.6's VFS share): the access check of local files (a 0o444 file on Unix,
//! the DOS `R` attribute on Windows), the read-only flags of `MemoryVfs` entries and its `WRITE` capability.

mod common;

use common::{host, lspec, mspec};
use hikasa_core::CaseSensitivity;
use hikasa_vfs::{Attrs, Capabilities, DosAttrs, LocalVfs, MemMeta, MemoryVfs, UnixMeta, Vfs};

#[test]
fn local_files_report_write_access() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");
    std::fs::write(&path, b"x").unwrap();
    let vfs = LocalVfs::default();
    let spec = lspec(&path);
    assert_eq!(vfs.writable(&spec).unwrap(), Some(true));
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms.clone()).unwrap();
    // A process that may write anyway (root on Unix) sees the file writable: the check follows the process's
    // access, not the mode bits.
    let can_write = std::fs::OpenOptions::new().write(true).open(&path).is_ok();
    assert_eq!(vfs.writable(&spec).unwrap(), Some(can_write));
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn memory_entries_follow_their_flags() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let unix = |mode| MemMeta {
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: 0,
                gid: 0,
            },
        }),
        ..MemMeta::default()
    };
    vfs.insert_file(&host("/rw.txt"), b"a".to_vec(), unix(0o644))
        .unwrap();
    vfs.insert_file(&host("/ro.txt"), b"a".to_vec(), unix(0o444))
        .unwrap();
    vfs.insert_file(
        &host("/dos.txt"),
        b"a".to_vec(),
        MemMeta {
            attrs: Some(Attrs::Dos {
                attrs: DosAttrs::READONLY,
            }),
            ..MemMeta::default()
        },
    )
    .unwrap();
    vfs.insert_file(&host("/plain.txt"), b"a".to_vec(), MemMeta::default())
        .unwrap();
    assert_eq!(vfs.writable(&mspec("/rw.txt")).unwrap(), Some(true));
    assert_eq!(vfs.writable(&mspec("/ro.txt")).unwrap(), Some(false));
    assert_eq!(vfs.writable(&mspec("/dos.txt")).unwrap(), Some(false));
    assert_eq!(vfs.writable(&mspec("/plain.txt")).unwrap(), Some(true));
    assert!(vfs.writable(&mspec("/missing.txt")).is_err());
    vfs.set_capabilities(vfs.capabilities(&mspec("/")).unwrap() - Capabilities::WRITE);
    assert_eq!(vfs.writable(&mspec("/rw.txt")).unwrap(), Some(false));
}
