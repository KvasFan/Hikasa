//! Content helpers (§2.20; FC-025, VFS-020): `VfsSource` (`read_all` limit → `TooLarge`, cancellation →
//! `ReadError::Cancelled`), the same-object and length shortcuts of `compare_sources`, the known-CRC shortcut of
//! `crc32_source`, `CrcComparator` and `BinaryComparator` results, errors and cancellation, and progress.

mod common;

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Mutex;

use common::{host, lspec, mspec};
use hikasa_core::{
    CancelToken, Classify, ContentComparator, ContentMethod, ContentResult, ContentSource, Ctl,
    ErrorKind, NullProgress, Phase, Progress, ReadError, Spec,
};
use hikasa_vfs::{
    BinaryComparator, BinaryOutcome, CrcComparator, Fault, FollowLinks, LocalVfs, MemMeta,
    MemoryVfs, MmapPolicy, StreamOptions, Vfs, VfsError, VfsOp, VfsSource, compare_sources,
    crc32_source,
};

fn source<'a>(vfs: &'a dyn Vfs, spec: Spec) -> VfsSource<'a> {
    let entry = vfs.stat(&spec, FollowLinks::Yes).unwrap();
    VfsSource::new(vfs, spec, entry)
}

fn memory(files: &[(&str, &[u8])]) -> MemoryVfs {
    let vfs = MemoryVfs::default();
    for (path, content) in files {
        vfs.insert_file(&host(path), content.to_vec(), MemMeta::default())
            .unwrap();
    }
    vfs
}

fn fault(vfs: &MemoryVfs, path: &str, op: VfsOp, kind: ErrorKind) {
    vfs.inject(Fault {
        path: host(path),
        op,
        kind,
    });
}

const STREAM: StreamOptions = StreamOptions {
    buffer_size: 4096,
    mmap: MmapPolicy::Never,
    threads: 1,
};

#[test]
fn vfs_source_describes_its_entry() {
    let vfs = memory(&[("/a/x.c", b"int x;\n")]);
    vfs.insert_dir(&host("/a/d"), MemMeta::default()).unwrap();
    let src = source(&vfs, mspec("/a/x.c"));
    assert_eq!(src.name(), "x.c");
    assert_eq!(src.display(), host("/a/x.c"));
    assert_eq!(src.len(), Some(7));
    assert_eq!(src.known_crc32(), None);
    assert_eq!(src.local_path(), None);
    assert_eq!(common::read_to_vec(src.open().unwrap()), b"int x;\n");
    assert_eq!(src.spec(), &mspec("/a/x.c"));
    assert_eq!(src.entry().size, 7);
    assert!(format!("{src:?}").starts_with("VfsSource"));
    let dir = source(&vfs, mspec("/a/d"));
    assert_eq!(dir.len(), None);
    // Local files have a local path; folders and missing paths do not.
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("f"), b"1").unwrap();
    let local = LocalVfs::default();
    let file = source(&local, lspec(&tmp.path().join("f")));
    assert_eq!(
        file.local_path(),
        Some(tmp.path().canonicalize().unwrap().join("f").as_path())
    );
    assert_eq!(source(&local, lspec(tmp.path())).local_path(), None);
}

#[test]
fn read_all_limits_cancellation_and_faults() {
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7) as u8).collect();
    let vfs = memory(&[("/big", &data), ("/small", b"abc")]);
    let big = source(&vfs, mspec("/big"));
    let never = CancelToken::never();
    let all = big.read_all(u64::MAX, &never).unwrap();
    assert_eq!(all, data);
    assert_eq!(all.capacity(), data.len(), "exact-size buffer");
    assert!(matches!(
        big.read_all(2_999_999, &never),
        Err(ReadError::TooLarge { limit: 2_999_999 })
    ));
    assert_eq!(big.read_all(3_000_000, &never).unwrap().len(), 3_000_000);
    let token = CancelToken::new();
    token.cancel();
    let err = big.read_all(u64::MAX, &token).unwrap_err();
    assert!(matches!(err, ReadError::Cancelled));
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    // A read fault surfaces as `ReadError::Io` of the fault's kind; an open fault likewise.
    fault(&vfs, "/small", VfsOp::Read, ErrorKind::InvalidData);
    let small = source(&vfs, mspec("/small"));
    let err = small.read_all(10, &never).unwrap_err();
    assert!(matches!(err, ReadError::Io(_)));
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    vfs.clear_faults();
    fault(&vfs, "/small", VfsOp::Open, ErrorKind::PermissionDenied);
    let err = small.read_all(10, &never).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    let open_err = small.open().map(|_| ()).unwrap_err();
    assert_eq!(
        hikasa_core::io_error_kind(&open_err),
        ErrorKind::PermissionDenied
    );
    // The original VfsError is recovered from the io::Error.
    let back = VfsError::from_io("ignored", VfsOp::Open, open_err);
    assert_eq!(
        back.to_string(),
        format!("{}: permission denied", host("/small"))
    );
}

#[test]
fn compare_sources_shortcuts_and_streaming() {
    let big = vec![b'x'; 5000];
    let longer = vec![b'x'; 6000];
    let vfs = memory(&[
        ("/a", b"hello world"),
        ("/b", b"hello world"),
        ("/c", b"hello there"),
        ("/d", b"hello"),
        ("/big", &big),
        ("/longer", &longer),
    ]);
    let ctl = Ctl::detached();
    let s = |p: &str| source(&vfs, mspec(p));
    assert_eq!(
        compare_sources(&s("/a"), &s("/b"), &STREAM, &ctl).unwrap(),
        BinaryOutcome::Same
    );
    assert_eq!(
        compare_sources(&s("/a"), &s("/c"), &STREAM, &ctl).unwrap(),
        BinaryOutcome::Different {
            first_difference: Some(6)
        }
    );
    // Small reported sizes (≤ 4096 bytes) are not trusted: the files are read and the offset is known.
    assert_eq!(
        compare_sources(&s("/a"), &s("/d"), &STREAM, &ctl).unwrap(),
        BinaryOutcome::Different {
            first_difference: Some(5)
        }
    );
    // Length shortcut for sizes above 4096 bytes: decided without opening either file.
    for p in ["/a", "/d", "/big", "/longer"] {
        fault(&vfs, p, VfsOp::Open, ErrorKind::PermissionDenied);
    }
    assert_eq!(
        compare_sources(&s("/big"), &s("/longer"), &STREAM, &ctl).unwrap(),
        BinaryOutcome::Different {
            first_difference: None
        }
    );
    // A small size is read even when the other one is large: the open fault surfaces.
    let err = compare_sources(&s("/d"), &s("/longer"), &STREAM, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    // Equal lengths must be read: the open fault surfaces with the failing spec.
    let err = compare_sources(&s("/b"), &s("/a"), &STREAM, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    assert_eq!(err.spec(), Some(host("/a").as_str()));
}

/// A source over `vfs` whose entry reports `size` instead of the content length (a pseudo-file: procfs reports
/// 0, sysfs 4096).
fn misreported<'a>(vfs: &'a dyn Vfs, path: &str, size: u64) -> VfsSource<'a> {
    let spec = mspec(path);
    let mut entry = vfs.stat(&spec, FollowLinks::Yes).unwrap();
    entry.size = size;
    VfsSource::new(vfs, spec, entry)
}

/// Regression (pseudo-files): a reported size of at most 4096 bytes never decides "different" by length alone;
/// both comparators read such files, so identical bytes are `BinarySame`.
#[test]
fn unreliable_small_sizes_are_read() {
    let vfs = memory(&[
        ("/proc/ostype", b"Linux\n"),
        ("/copy/ostype", b"Linux\n"),
        ("/sys/enabled", b"always [madvise] never\n"),
        ("/copy/enabled", b"always [madvise] never\n"),
        ("/copy/other", b"always madvise [never]\n"),
    ]);
    let ctl = Ctl::detached();
    let crc = CrcComparator { opts: STREAM };
    let binary = BinaryComparator { opts: STREAM };
    let cases = [
        // procfs: size 0, content 6 bytes.
        ("/proc/ostype", 0, "/copy/ostype", ContentResult::BinarySame),
        // sysfs: size 4096, content 23 bytes.
        (
            "/sys/enabled",
            4096,
            "/copy/enabled",
            ContentResult::BinarySame,
        ),
        (
            "/sys/enabled",
            4096,
            "/copy/other",
            ContentResult::BinaryDifferent,
        ),
    ];
    for (pseudo, size, copy, expected) in cases {
        let (a, b) = (misreported(&vfs, pseudo, size), source(&vfs, mspec(copy)));
        assert_ne!(a.len(), b.len(), "the reported sizes differ");
        for comparator in [&crc as &dyn ContentComparator, &binary] {
            assert_eq!(
                comparator.compare(&a, &b, &ctl).unwrap().result,
                expected,
                "{pseudo}"
            );
            assert_eq!(
                comparator.compare(&b, &a, &ctl).unwrap().result,
                expected,
                "{pseudo}"
            );
        }
        let outcome = compare_sources(&a, &b, &StreamOptions::default(), &ctl).unwrap();
        assert_eq!(outcome.is_same(), expected == ContentResult::BinarySame);
        // `read_all` does not trust the small size against its limit either.
        assert_eq!(
            a.read_all(100, &CancelToken::never()).unwrap().len() as u64,
            b.len().unwrap()
        );
    }
    // Above 4096 bytes the length still decides (nothing is opened).
    let big = vec![1u8; 10_000];
    vfs.insert_file(&host("/big"), big, MemMeta::default())
        .unwrap();
    fault(&vfs, "/big", VfsOp::Open, ErrorKind::PermissionDenied);
    let (a, b) = (misreported(&vfs, "/big", 4097), source(&vfs, mspec("/big")));
    for comparator in [&crc as &dyn ContentComparator, &binary] {
        assert_eq!(
            comparator.compare(&a, &b, &ctl).unwrap().result,
            ContentResult::BinaryDifferent
        );
    }
    assert!(matches!(
        a.read_all(100, &CancelToken::never()),
        Err(ReadError::TooLarge { limit: 100 })
    ));
}

/// Regression on Linux: real procfs and sysfs files (when present) equal their copies under both comparators.
#[cfg(target_os = "linux")]
#[test]
fn linux_pseudo_files_equal_their_copies() {
    let local = LocalVfs::default();
    let tmp = tempfile::tempdir().unwrap();
    let ctl = Ctl::detached();
    let crc = CrcComparator {
        opts: StreamOptions::default(),
    };
    let binary = BinaryComparator {
        opts: StreamOptions::default(),
    };
    for pseudo in [
        "/proc/sys/kernel/ostype",
        "/sys/kernel/mm/transparent_hugepage/enabled",
    ] {
        let path = Path::new(pseudo);
        let Ok(content) = fs::read(path) else {
            continue;
        };
        let copy = tmp.path().join("copy");
        fs::write(&copy, &content).unwrap();
        let a = source(
            &local,
            Spec::Local(hikasa_core::LocalPath::new(path.into()).unwrap()),
        );
        let b = source(&local, lspec(&copy));
        for comparator in [&crc as &dyn ContentComparator, &binary] {
            assert_eq!(
                comparator.compare(&a, &b, &ctl).unwrap().result,
                ContentResult::BinarySame,
                "{pseudo}: reported size {:?}, content {} bytes",
                a.len(),
                content.len()
            );
        }
    }
}

/// A local file that refuses to be opened: only the same-object shortcut can compare it.
struct Unopenable<'a> {
    path: &'a Path,
}

impl ContentSource for Unopenable<'_> {
    fn name(&self) -> &str {
        "f"
    }
    fn display(&self) -> String {
        self.path.display().to_string()
    }
    fn len(&self) -> Option<u64> {
        Some(7)
    }
    fn local_path(&self) -> Option<&Path> {
        Some(self.path)
    }
    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Err(io::Error::other("must not be opened"))
    }
    fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Err(ReadError::Io(io::Error::other("must not be read")))
    }
}

#[test]
fn same_object_shortcut_reads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    fs::write(&path, b"content").unwrap();
    let other = dir.path().join("g");
    fs::write(&other, b"content").unwrap();
    let a = Unopenable { path: &path };
    let b = Unopenable { path: &path };
    for opts in [STREAM, StreamOptions::default()] {
        assert_eq!(
            compare_sources(&a, &b, &opts, &Ctl::detached()).unwrap(),
            BinaryOutcome::Same
        );
    }
    // Different objects with equal lengths must be read (and here cannot be).
    let c = Unopenable { path: &other };
    let err = compare_sources(&a, &c, &STREAM, &Ctl::detached()).unwrap_err();
    assert!(err.to_string().contains("must not be opened"), "{err}");
    #[cfg(unix)]
    {
        // A symlink alias is the same object.
        std::os::unix::fs::symlink("f", dir.path().join("alias")).unwrap();
        let alias = dir.path().join("alias");
        let d = Unopenable { path: &alias };
        assert_eq!(
            compare_sources(&a, &d, &STREAM, &Ctl::detached()).unwrap(),
            BinaryOutcome::Same
        );
    }
}

#[test]
fn mapped_and_streamed_local_comparisons_agree() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = common::XorShift(0x5EED);
    let base = rng.bytes(200_000);
    let mut changed = base.clone();
    changed[123_457] ^= 0x40;
    let write = |name: &str, data: &[u8]| {
        let path = dir.path().join(name);
        fs::write(&path, data).unwrap();
        path
    };
    let (pa, pb, pc) = (write("a", &base), write("b", &base), write("c", &changed));
    let vfs = LocalVfs::default();
    let mapped = StreamOptions {
        buffer_size: 4096,
        mmap: MmapPolicy::Auto { min_len: 0 },
        threads: 3,
    };
    let ctl = Ctl::detached();
    for opts in [STREAM, mapped, StreamOptions::default()] {
        let s = |p: &Path| source(&vfs, lspec(p));
        assert_eq!(
            compare_sources(&s(&pa), &s(&pb), &opts, &ctl).unwrap(),
            BinaryOutcome::Same
        );
        assert_eq!(
            compare_sources(&s(&pa), &s(&pc), &opts, &ctl).unwrap(),
            BinaryOutcome::Different {
                first_difference: Some(123_457)
            }
        );
        assert_eq!(
            crc32_source(&s(&pc), &opts, &ctl).unwrap(),
            crc32fast::hash(&changed)
        );
    }
}

#[test]
fn crc32_source_prefers_the_known_crc() {
    let vfs = MemoryVfs::default();
    vfs.insert_file(
        &host("/archive/member"),
        b"payload".to_vec(),
        MemMeta {
            crc32: Some(0x1234_5678),
            ..MemMeta::default()
        },
    )
    .unwrap();
    vfs.insert_file(&host("/plain"), b"payload".to_vec(), MemMeta::default())
        .unwrap();
    fault(&vfs, "/archive/member", VfsOp::Open, ErrorKind::Io);
    let ctl = Ctl::detached();
    let member = source(&vfs, mspec("/archive/member"));
    assert_eq!(member.known_crc32(), Some(0x1234_5678));
    assert_eq!(crc32_source(&member, &STREAM, &ctl).unwrap(), 0x1234_5678);
    assert_eq!(
        crc32_source(&source(&vfs, mspec("/plain")), &STREAM, &ctl).unwrap(),
        crc32fast::hash(b"payload")
    );
}

#[test]
fn comparators_report_results_errors_and_cancellation() {
    let vfs = memory(&[
        ("/l", b"same bytes"),
        ("/r", b"same bytes"),
        ("/x", b"diff bytes"),
        ("/short", b"same"),
    ]);
    let ctl = Ctl::detached();
    let s = |p: &str| source(&vfs, mspec(p));
    let crc = CrcComparator { opts: STREAM };
    let binary = BinaryComparator { opts: STREAM };
    assert_eq!(crc.method(), ContentMethod::Crc);
    assert_eq!(binary.method(), ContentMethod::Binary);
    assert_eq!(crc.size_limit(), None);
    assert_eq!(binary.size_limit(), None);
    for comparator in [&crc as &dyn ContentComparator, &binary] {
        let same = comparator.compare(&s("/l"), &s("/r"), &ctl).unwrap();
        assert_eq!(same.result, ContentResult::BinarySame);
        assert_eq!(same.method, comparator.method());
        assert!(same.warnings.is_empty());
        assert_eq!(
            comparator.compare(&s("/l"), &s("/x"), &ctl).unwrap().result,
            ContentResult::BinaryDifferent
        );
        assert_eq!(
            comparator
                .compare(&s("/l"), &s("/short"), &ctl)
                .unwrap()
                .result,
            ContentResult::BinaryDifferent
        );
        let token = CancelToken::new();
        token.cancel();
        let cancelled = Ctl {
            cancel: &token,
            progress: &NullProgress,
        };
        assert!(comparator.compare(&s("/l"), &s("/r"), &cancelled).is_err());
    }
    // Errors become `ContentResult::Error` naming the failed side.
    fault(&vfs, "/r", VfsOp::Read, ErrorKind::InvalidData);
    for comparator in [&crc as &dyn ContentComparator, &binary] {
        let outcome = comparator.compare(&s("/l"), &s("/r"), &ctl).unwrap();
        let ContentResult::Error { error } = outcome.result else {
            panic!("expected an error, got {:?}", outcome.result);
        };
        assert_eq!(error.kind, ErrorKind::InvalidData);
        assert_eq!(error.spec.as_deref(), Some(host("/r").as_str()));
        assert!(
            error.message.contains("injected fault"),
            "{}",
            error.message
        );
    }
    // A cancelled read (an io::Error that wraps `Cancelled`) is cancellation, not an error result.
    vfs.clear_faults();
    fault(&vfs, "/r", VfsOp::Read, ErrorKind::Cancelled);
    assert!(crc.compare(&s("/l"), &s("/r"), &ctl).is_err());
    assert!(binary.compare(&s("/l"), &s("/r"), &ctl).is_err());
}

/// A source whose reader cancels the token after its first read: the next per-buffer check returns.
struct CancellingSource<'a> {
    data: Vec<u8>,
    token: &'a CancelToken,
}

struct CancellingReader<'a> {
    inner: io::Cursor<Vec<u8>>,
    token: &'a CancelToken,
}

impl Read for CancellingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.token.cancel();
        self.inner.read(buf)
    }
}

impl ContentSource for CancellingSource<'_> {
    fn name(&self) -> &str {
        "cancel"
    }
    fn display(&self) -> String {
        "cancel".into()
    }
    fn len(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(CancellingReader {
            inner: io::Cursor::new(self.data.clone()),
            token: self.token,
        }))
    }
    fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Ok(self.data.clone())
    }
}

#[test]
fn cancellation_is_checked_per_buffer() {
    let token = CancelToken::new();
    let ctl = Ctl {
        cancel: &token,
        progress: &NullProgress,
    };
    let a = CancellingSource {
        data: vec![1; 100_000],
        token: &token,
    };
    let b = CancellingSource {
        data: vec![1; 100_000],
        token: &token,
    };
    assert!(matches!(
        compare_sources(&a, &b, &STREAM, &ctl),
        Err(VfsError::Cancelled)
    ));
    let token2 = CancelToken::new();
    let c = CancellingSource {
        data: vec![1; 100_000],
        token: &token2,
    };
    let ctl2 = Ctl {
        cancel: &token2,
        progress: &NullProgress,
    };
    assert!(matches!(
        crc32_source(&c, &STREAM, &ctl2),
        Err(VfsError::Cancelled)
    ));
}

#[test]
fn progress_is_reported_in_bytes() {
    let data = vec![3u8; 50_000];
    let vfs = memory(&[("/a", &data), ("/b", &data)]);
    let seen: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
    let sink = |p: Progress| seen.lock().unwrap().push(p);
    let token = CancelToken::never();
    let ctl = Ctl {
        cancel: &token,
        progress: &sink,
    };
    let s = |p: &str| source(&vfs, mspec(p));
    compare_sources(&s("/a"), &s("/b"), &STREAM, &ctl).unwrap();
    crc32_source(&s("/a"), &STREAM, &ctl).unwrap();
    let seen = seen.into_inner().unwrap();
    for phase in [Phase::Reading, Phase::Hashing] {
        let done: Vec<u64> = seen
            .iter()
            .filter(|p| p.phase == phase)
            .map(|p| p.done)
            .collect();
        assert!(!done.is_empty(), "{phase:?}");
        assert!(done.windows(2).all(|w| w[0] <= w[1]), "never decreases");
        assert_eq!(*done.last().unwrap(), 50_000);
    }
    assert!(
        seen.iter()
            .all(|p| p.total == Some(50_000) && p.current.is_none())
    );
}

/// Regression: cancellation that arrives as an error (an open fault or a reader error of kind `cancelled`) is
/// `ReadError::Cancelled`, not `ReadError::Io`.
#[test]
fn read_all_reports_cancellation_errors_as_cancelled() {
    let vfs = memory(&[("/f", b"content")]);
    let never = CancelToken::never();
    for op in [VfsOp::Open, VfsOp::Read] {
        vfs.clear_faults();
        fault(&vfs, "/f", op, ErrorKind::Cancelled);
        let err = source(&vfs, mspec("/f")).read_all(100, &never).unwrap_err();
        assert!(matches!(err, ReadError::Cancelled), "{op:?}: {err:?}");
    }
}

/// Regression: `VfsSource::new` makes no back-end call; the local path is asked once, on first use, and a CRC
/// of a file too small to be mapped never asks for it.
#[test]
fn local_path_is_asked_lazily_and_once() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("small"), b"tiny").unwrap();
    fs::write(tmp.path().join("other"), b"tiny").unwrap();
    let local = LocalVfs::default();
    let counting = common::CountingVfs::new(&local);
    let stat = |p: &str| {
        let spec = lspec(&tmp.path().join(p));
        let entry = local.stat(&spec, FollowLinks::Yes).unwrap();
        VfsSource::new(&counting, spec, entry)
    };
    let (a, b) = (stat("small"), stat("other"));
    assert_eq!(counting.local_paths(), 0, "no call in VfsSource::new");
    let ctl = Ctl::detached();
    let opts = StreamOptions::default();
    assert_eq!(
        crc32_source(&a, &opts, &ctl).unwrap(),
        crc32fast::hash(b"tiny")
    );
    assert_eq!(counting.local_paths(), 0, "too small to be mapped");
    assert!(compare_sources(&a, &b, &opts, &ctl).unwrap().is_same());
    assert_eq!(
        counting.local_paths(),
        2,
        "the same-object test asks once per source"
    );
    assert!(a.local_path().is_some() && b.local_path().is_some());
    assert!(compare_sources(&a, &b, &opts, &ctl).unwrap().is_same());
    assert_eq!(counting.local_paths(), 2, "cached");
}

/// Regression: the completion report (`done == total`) of every path passes a throttle that is closed for
/// everything else (a `ThrottledSink` with a long interval forwards the first report of a phase and
/// completions only).
#[test]
fn completion_passes_the_throttle() {
    let tmp = tempfile::tempdir().unwrap();
    let data = common::XorShift(7).bytes(300_000);
    fs::write(tmp.path().join("a"), &data).unwrap();
    fs::write(tmp.path().join("b"), &data).unwrap();
    let local = LocalVfs::default();
    let s = |p: &str| source(&local, lspec(&tmp.path().join(p)));
    for (mmap, threads) in [
        (MmapPolicy::Never, 1),
        (MmapPolicy::Auto { min_len: 0 }, 1),
        (MmapPolicy::Auto { min_len: 0 }, 4),
    ] {
        let seen: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        let record = |p: Progress| seen.lock().unwrap().push(p);
        let sink = hikasa_core::ThrottledSink::new(&record, std::time::Duration::from_secs(3600));
        let token = CancelToken::never();
        let ctl = Ctl {
            cancel: &token,
            progress: &sink,
        };
        let opts = StreamOptions {
            buffer_size: 4096,
            mmap,
            threads,
        };
        assert!(
            compare_sources(&s("a"), &s("b"), &opts, &ctl)
                .unwrap()
                .is_same()
        );
        assert_eq!(
            crc32_source(&s("a"), &opts, &ctl).unwrap(),
            crc32fast::hash(&data)
        );
        let seen = seen.lock().unwrap();
        for phase in [Phase::Reading, Phase::Hashing] {
            let last = seen.iter().rfind(|p| p.phase == phase).unwrap();
            assert_eq!(
                (last.done, last.total),
                (300_000, Some(300_000)),
                "{phase:?} {mmap:?} {threads}"
            );
        }
    }
}

/// A reader that returns 64 KiB per call and cancels `token` on its third call.
struct SlowCancelling<'a> {
    calls: usize,
    token: &'a CancelToken,
}

impl Read for SlowCancelling<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 3 {
            self.token.cancel();
        }
        let n = buf.len().min(64 << 10);
        buf[..n].fill(1);
        Ok(n)
    }
}

/// Regression (slow readers): the public reader helpers stop at the first partial read after `cancel()`, not
/// after the whole 1 MiB (or 64 MiB) buffer.
#[test]
fn reader_helpers_stop_between_partial_reads() {
    for buffer_size in [1 << 20, 64 << 20] {
        let opts = StreamOptions {
            buffer_size,
            ..StreamOptions::default()
        };
        let token = CancelToken::new();
        let ctl = Ctl {
            cancel: &token,
            progress: &NullProgress,
        };
        let mut slow = SlowCancelling {
            calls: 0,
            token: &token,
        };
        let mut other = io::repeat(1).take(1 << 30);
        assert!(matches!(
            hikasa_vfs::compare_readers(&mut slow, &mut other, &opts, &ctl),
            Err(VfsError::Cancelled)
        ));
        assert_eq!(slow.calls, 3);
        let token = CancelToken::new();
        let ctl = Ctl {
            cancel: &token,
            progress: &NullProgress,
        };
        let mut slow = SlowCancelling {
            calls: 0,
            token: &token,
        };
        assert!(matches!(
            hikasa_vfs::crc32_reader(&mut slow, &opts, &ctl),
            Err(VfsError::Cancelled)
        ));
        assert_eq!(slow.calls, 3);
    }
    // `read_all` likewise: see the unit test `source::tests::cancellation_between_partial_reads`.
}
