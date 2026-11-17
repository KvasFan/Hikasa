//! A12.2 (D-326): the hex document.
//!
//! - reads at every page edge and across pages equal the file bytes (`File` load: the page cache);
//! - a file of at most `unlocked_max` bytes is `Memory` and its handle is closed (Linux: `/proc/self/fd` holds no
//!   entry for it; every platform: the file can be replaced while the document is open), a larger one is `File`
//!   (and its handle held);
//! - a member of another back-end is `TempCache` and reads equal the back-end's bytes; standard input and clipboard
//!   bytes are `Memory`;
//! - a truncation is `Truncated`; a changed modification time gives `Changed` (`input_changed`).

mod common;

use std::path::PathBuf;

use common::{XorShift, lspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Classify, Ctl, ErrorKind, FileTime, LocalPath, Spec,
};
use hikasa_hex::{HexDocument, HexError, HexLoad, HexLoadOptions, HexWarningCode};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, FollowLinks, ListOptions, Listing, LocalVfs, MemMeta,
    MemoryVfs, ReadSeek, Vfs, VfsError, WriteOptions,
};

const PAGE: u64 = 64 << 10;

fn file_load() -> HexLoadOptions {
    HexLoadOptions {
        unlocked_max: 0,
        ..HexLoadOptions::default()
    }
}

#[test]
fn reads_at_every_page_edge_equal_the_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    let data = XorShift(3).bytes((10 * PAGE + 123) as usize);
    std::fs::write(&path, &data).unwrap();
    let vfs = LocalVfs::default();
    let load = HexLoadOptions {
        cache_pages: 3,
        ..file_load()
    };
    let d = HexDocument::open(&vfs, &lspec(&path), &load, &Ctl::detached()).unwrap();
    assert_eq!(d.load(), HexLoad::File);
    assert_eq!(d.len(), data.len() as u64);
    for page in 0..=11u64 {
        for delta in [-17i64, -1, 0, 1, 5000] {
            let at = (page * PAGE) as i64 + delta;
            if at < 0 {
                continue;
            }
            let at = at as u64;
            for len in [1usize, 2, 100, PAGE as usize + 3, 3 * PAGE as usize] {
                let mut buf = vec![0u8; len];
                let n = d.read(at, &mut buf).unwrap();
                let end = (at as usize + len).min(data.len());
                let want = data.get(at as usize..end).unwrap_or_default();
                assert_eq!(&buf[..n], want, "at {at} len {len}");
            }
        }
    }
    assert!(d.is_cached(10 * PAGE..10 * PAGE + 10));
    assert!(!d.is_cached(0..1));
    assert!(d.is_cached(d.len()..d.len() + 5));
}

/// The open file descriptors of this process that name `path` (Linux).
#[cfg(target_os = "linux")]
fn open_handles(path: &std::path::Path) -> usize {
    let target = std::fs::canonicalize(path).unwrap();
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
        .filter(|link| *link == target)
        .count()
}

#[test]
fn small_files_are_read_whole_and_their_handle_closed() {
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small");
    let large = dir.path().join("large");
    std::fs::write(&small, vec![1u8; 1000]).unwrap();
    std::fs::write(&large, vec![2u8; 3000]).unwrap();
    let vfs = LocalVfs::default();
    let load = HexLoadOptions {
        unlocked_max: 2000,
        ..HexLoadOptions::default()
    };
    let ds = HexDocument::open(&vfs, &lspec(&small), &load, &Ctl::detached()).unwrap();
    let dl = HexDocument::open(&vfs, &lspec(&large), &load, &Ctl::detached()).unwrap();
    assert_eq!((ds.load(), dl.load()), (HexLoad::Memory, HexLoad::File));
    #[cfg(target_os = "linux")]
    {
        assert_eq!(open_handles(&small), 0, "the small file is not held");
        assert_eq!(open_handles(&large), 1, "the large file keeps one handle");
    }
    // The small file can be replaced while its document is open (on Windows an open handle would refuse it).
    let fresh = dir.path().join("fresh");
    std::fs::write(&fresh, b"new").unwrap();
    std::fs::rename(&fresh, &small).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(ds.read(0, &mut buf).unwrap(), 4);
    assert_eq!(buf, [1; 4], "the document keeps the bytes it read");
    // (Not asserted: that the kept handle blocks a replace on Windows. The standard library opens files with
    // FILE_SHARE_DELETE, so a replace succeeds there too; contract §1.12.33 W12a note 23.)
    drop((ds, dl));
}

#[test]
fn a_truncation_is_an_error_and_a_change_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    std::fs::write(&path, vec![5u8; 4 * PAGE as usize]).unwrap();
    let vfs = LocalVfs::default();
    let d = HexDocument::open(&vfs, &lspec(&path), &file_load(), &Ctl::detached()).unwrap();
    assert_eq!(d.check_unchanged(&vfs), Ok(()));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(PAGE + 10)
        .unwrap();
    let mut buf = [0u8; 100];
    assert_eq!(d.read(0, &mut buf).unwrap(), 100);
    let err = d.read(3 * PAGE, &mut buf).unwrap_err();
    assert!(matches!(err, HexError::Truncated { .. }), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    let changed = d.check_unchanged(&vfs).unwrap_err();
    assert_eq!(changed.warning().code, HexWarningCode::InputChanged);
    // A new modification time alone is a change too.
    let other = dir.path().join("g");
    std::fs::write(&other, b"same").unwrap();
    let g = HexDocument::open(
        &vfs,
        &lspec(&other),
        &HexLoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let f = std::fs::File::options().write(true).open(&other).unwrap();
    f.set_modified(
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000),
    )
    .unwrap();
    assert!(g.check_unchanged(&vfs).is_err());
    // A file that was a folder is refused.
    let err =
        HexDocument::open(&vfs, &lspec(dir.path()), &file_load(), &Ctl::detached()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::IsADirectory);
    // A missing file names its spec.
    let missing = dir.path().join("missing");
    let err =
        HexDocument::open(&vfs, &lspec(&missing), &file_load(), &Ctl::detached()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.spec().unwrap().contains("missing"));
}

fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(host(path))).unwrap())
}

/// A back-end that serves `ArchiveMember` specs from a `MemoryVfs` (as `<archive>/<member>`): the document must
/// read them as `TempCache`.
struct Members(MemoryVfs);

impl Members {
    fn resolve(spec: &Spec) -> Spec {
        match spec {
            Spec::ArchiveMember { archive, member } => {
                let Spec::Local(base) = archive.as_ref() else {
                    panic!()
                };
                let path = member
                    .split('/')
                    .fold(base.as_path().to_path_buf(), |p, c| p.join(c));
                Spec::Local(LocalPath::new(path).unwrap())
            }
            other => other.clone(),
        }
    }
}

impl Vfs for Members {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.0.capabilities(&Self::resolve(at))
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.0.stat(&Self::resolve(spec), follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.0.list(&Self::resolve(dir), opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.0.open_read(&Self::resolve(spec))
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.0.open_write(&Self::resolve(spec), opts)
    }
    fn child(&self, dir: &Spec, name: &hikasa_vfs::FileName) -> Result<Spec, VfsError> {
        self.0.child(dir, name)
    }
}

#[test]
fn members_are_temp_cache_and_blobs_memory() {
    let mem = MemoryVfs::new(CaseSensitivity::Sensitive);
    let data = XorShift(9).bytes(200_000);
    let meta = MemMeta {
        mtime: FileTime::from_unix(1_700_000_000, 0),
        ..MemMeta::default()
    };
    mem.insert_file(&host("/a.zip/inner/m.bin"), data.clone(), meta)
        .unwrap();
    let vfs = Members(mem);
    let spec = Spec::archive_member(mspec("/a.zip"), "inner/m.bin");
    let d = HexDocument::open(&vfs, &spec, &HexLoadOptions::default(), &Ctl::detached()).unwrap();
    assert_eq!(d.load(), HexLoad::TempCache);
    assert!(!d.info().converted);
    let mut all = vec![0u8; data.len()];
    assert_eq!(d.read(0, &mut all).unwrap(), data.len());
    assert_eq!(all, data);
    assert_eq!(d.info().mtime, FileTime::from_unix(1_700_000_000, 0));
    // A local file of MemoryVfs within the limit is Memory.
    let local = HexDocument::open(
        &vfs.0,
        &mspec("/a.zip/inner/m.bin"),
        &HexLoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(local.load(), HexLoad::Memory);
    // Bytes of standard input or the clipboard.
    let blob = HexDocument::from_bytes("-", data.clone().into());
    assert_eq!(blob.load(), HexLoad::Memory);
    assert_eq!(blob.check_unchanged(&vfs), Ok(()));
}

#[test]
fn cancellation_while_reading_whole() {
    let mem = MemoryVfs::new(CaseSensitivity::Sensitive);
    mem.insert_file(&host("/f"), vec![0u8; 3 << 20], MemMeta::default())
        .unwrap();
    let token = common::cancelled();
    let ctl = Ctl {
        cancel: &token,
        progress: &hikasa_core::NullProgress,
    };
    let err = HexDocument::open(&mem, &mspec("/f"), &HexLoadOptions::default(), &ctl).unwrap_err();
    assert!(err.is_cancelled());
}

/// A12.31: a file whose name is not UTF-8 (Linux) opens and reads like any other.
#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_name_opens() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xE9.bin"));
    std::fs::write(&path, b"bytes").unwrap();
    for load in [HexLoadOptions::default(), file_load()] {
        let d = HexDocument::open(&LocalVfs::default(), &lspec(&path), &load, &Ctl::detached())
            .unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(d.read(0, &mut buf).unwrap(), 5);
        assert_eq!(&buf, b"bytes");
    }
}

/// W12 review F2: a read at or past the end returns 0 (never panics) for every load, and so does an empty buffer.
#[test]
fn reads_at_and_past_the_end_return_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    std::fs::write(&path, b"abc").unwrap();
    let vfs = LocalVfs::default();
    let memory = HexDocument::open(
        &vfs,
        &lspec(&path),
        &HexLoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let file = HexDocument::open(&vfs, &lspec(&path), &file_load(), &Ctl::detached()).unwrap();
    let blob = common::doc("d", b"abc");
    assert_eq!(
        (memory.load(), file.load(), blob.load()),
        (HexLoad::Memory, HexLoad::File, HexLoad::Memory)
    );
    for d in [&memory, &file, &blob] {
        let mut buf = [9u8; 4];
        for offset in [3, 4, 5, 1 << 40, u64::MAX] {
            assert_eq!(d.read(offset, &mut buf).unwrap(), 0, "{offset}");
        }
        assert_eq!(d.read(1, &mut []).unwrap(), 0);
        assert_eq!(d.read(2, &mut buf).unwrap(), 1);
        assert_eq!(buf, [b'c', 9, 9, 9]);
    }
}

/// W12 review F5 (HX-009): an input whose reported size is at most 4096 bytes is read to its end, and its length
/// is the bytes read (bounded by `unlocked_max`, at least 4096); a member of another back-end too.
#[test]
fn unreliable_sizes_are_read_to_the_end() {
    let mem = MemoryVfs::new(CaseSensitivity::Sensitive);
    let data = XorShift(3).bytes(10_000);
    mem.insert_file(&host("/p/ten"), data.clone(), MemMeta::default())
        .unwrap();
    mem.insert_file(&host("/p/four"), data[..4096].to_vec(), MemMeta::default())
        .unwrap();
    mem.insert_file(&host("/p/four1"), data[..4097].to_vec(), MemMeta::default())
        .unwrap();
    mem.insert_file(&host("/p/empty"), Vec::new(), MemMeta::default())
        .unwrap();
    let ctl = Ctl::detached();
    for size in [0, 1, 4096] {
        let vfs = common::Pseudo { inner: &mem, size };
        let open = |name: &str, unlocked_max: u64| {
            let load = HexLoadOptions {
                unlocked_max,
                ..HexLoadOptions::default()
            };
            HexDocument::open(&vfs, &mspec(&format!("/p/{name}")), &load, &ctl)
        };
        for (name, want, unlocked_max) in [
            ("ten", &data[..], 10_000),
            ("ten", &data[..], HexLoadOptions::DEFAULT_UNLOCKED_MAX),
            ("four", &data[..4096], 0),
            ("empty", &[][..], 0),
        ] {
            let d = open(name, unlocked_max).unwrap();
            assert_eq!(d.len(), want.len() as u64, "{name} reported {size}");
            let load = if size <= unlocked_max {
                HexLoad::Memory
            } else {
                HexLoad::File
            };
            assert_eq!(d.load(), load, "{name} reported {size}");
            let mut all = vec![0u8; want.len() + 1];
            assert_eq!(d.read(0, &mut all).unwrap(), want.len());
            assert_eq!(&all[..want.len()], want);
            // The stamp is the reported size: an unchanged pseudo-file stays unchanged.
            assert_eq!(d.check_unchanged(&vfs), Ok(()));
        }
        // More than the bound (unlocked_max, at least 4096) is refused, never cut.
        for (name, unlocked_max) in [("ten", 9_999), ("four1", 0), ("four1", 4096)] {
            let err = open(name, unlocked_max).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::TooLarge, "{name} {unlocked_max}");
            assert!(err.spec().unwrap().contains(name));
        }
        // A cancelled read is cancelled.
        let token = common::cancelled();
        let cancelled = Ctl {
            cancel: &token,
            progress: &hikasa_core::NullProgress,
        };
        let err = HexDocument::open(
            &vfs,
            &mspec("/p/ten"),
            &HexLoadOptions::default(),
            &cancelled,
        )
        .unwrap_err();
        assert!(err.is_cancelled());
    }
    // A paged load (a member of another back-end; a local file of a reported size above `unlocked_max`) measures
    // its length by reading to its end, with the same bound.
    mem.insert_file(&host("/p/three"), data[..3000].to_vec(), MemMeta::default())
        .unwrap();
    let members = Members(mem);
    let member = Spec::archive_member(mspec("/p"), "ten");
    for (spec, size, unlocked_max, load, want) in [
        (
            &member,
            0,
            HexLoadOptions::DEFAULT_UNLOCKED_MAX,
            HexLoad::TempCache,
            &data[..],
        ),
        (&mspec("/p/three"), 4096, 100, HexLoad::File, &data[..3000]),
    ] {
        let pseudo = common::Pseudo {
            inner: &members,
            size,
        };
        let opts = HexLoadOptions {
            unlocked_max,
            ..HexLoadOptions::default()
        };
        let d = HexDocument::open(&pseudo, spec, &opts, &ctl).unwrap();
        assert_eq!((d.len(), d.load()), (want.len() as u64, load), "{spec:?}");
        let mut all = vec![0u8; want.len() + 1];
        assert_eq!(d.read(0, &mut all).unwrap(), want.len());
        assert_eq!(&all[..want.len()], want);
        assert_eq!(d.check_unchanged(&pseudo), Ok(()));
    }
    let pseudo = common::Pseudo {
        inner: &members,
        size: 0,
    };
    let opts = HexLoadOptions {
        unlocked_max: 9_999,
        ..HexLoadOptions::default()
    };
    let err = HexDocument::open(&pseudo, &member, &opts, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge);
}
