//! A5.1 (P-ARCH-1) and A5.2: spec resolution (§1.5.4): every row through `resolve_with` and a table-driven fake, in
//! both path styles, on every platform; again through `resolve_spec` on `ArchiveVfs` over `MemoryVfs` and `LocalVfs`.

mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use common::archives::{self, compress, default_masks, on_disk, on_memory, tar_bytes, zip_bytes};
use common::{host, mspec};
use hikasa_core::{CancelToken, PathStyle, Spec, SpecContext};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveMasks, MemMeta, Probe, Probed, SpecUse, Vfs, resolve_spec,
    resolve_with,
};
use proptest::prelude::*;

/// What the table of §1.5.4 assumes on disk.
struct Fake {
    kinds: BTreeMap<String, Probed>,
    masks: ArchiveMasks,
    bad: Vec<String>,
    opened: RefCell<Vec<String>>,
}

impl Fake {
    /// `/w` and the rows' files; `sep` joins paths on the disk, members always with `/`.
    fn new(root: &str, sep: &str) -> Fake {
        let path = |rest: &str| format!("{root}{sep}{}", rest.replace('/', sep));
        let member = |file: &str, rest: &str| format!("member {}/{rest}", path(file));
        let mut kinds = BTreeMap::new();
        let mut put = |key: String, kind| {
            kinds.insert(key, kind);
        };
        put(format!("disk {root}"), Probed::Dir);
        for file in [
            "a.zip",
            "plain.txt",
            "fake.zip",
            "up.ZIP",
            "link.zip",
            "x.tar.gz",
        ] {
            put(format!("disk {}", path(file)), Probed::File);
        }
        put(format!("disk {}", path("dir.zip")), Probed::Dir);
        put(format!("disk {}", path("dir.zip/y")), Probed::File);
        put(member("a.zip", "inner"), Probed::Dir);
        put(member("a.zip", "inner/x"), Probed::File);
        put(member("a.zip", "x"), Probed::File);
        put(member("a.zip", "b.tar.gz"), Probed::File);
        put(
            format!("member {}/b.tar.gz/c.txt", path("a.zip")),
            Probed::File,
        );
        put(member("a.zip", "lnk"), Probed::Other);
        put(member("up.ZIP", "x"), Probed::File);
        put(member("link.zip", "x"), Probed::File);
        Fake {
            kinds,
            masks: default_masks(),
            bad: vec![path("fake.zip")],
            opened: RefCell::new(Vec::new()),
        }
    }
}

impl Probe for Fake {
    fn kind(&self, spec: &Spec) -> Probed {
        let place = match spec {
            Spec::Local(_) => "disk",
            _ => "member",
        };
        let text = format!("{place} {}", spec.canonical());
        self.kinds.get(&text).copied().unwrap_or(Probed::Missing)
    }

    fn format(&self, name: &str) -> Option<ArchiveFormat> {
        self.masks.detect(name)
    }

    fn open(&self, archive: &Spec) -> bool {
        let text = archive.canonical();
        self.opened.borrow_mut().push(text.clone());
        !self.bad.contains(&text)
    }
}

/// `(input, use, chain)`: the chain is the archives from the outermost file in, then the member; empty = unchanged.
type Row = (&'static str, SpecUse, &'static [&'static str]);

/// The rows of §1.5.4 that hold in both path styles (paths below `/w` with `/` as written here).
const ROWS: &[Row] = &[
    ("a.zip", SpecUse::Any, &[]),
    ("a.zip", SpecUse::Folder, &["a.zip", ""]),
    ("a.zip/inner/x", SpecUse::Any, &["a.zip", "inner/x"]),
    ("a.zip/inner", SpecUse::Any, &["a.zip", "inner"]),
    ("a.zip/missing", SpecUse::Any, &["a.zip", "missing"]),
    (
        "a.zip/b.tar.gz/c.txt",
        SpecUse::Any,
        &["a.zip", "b.tar.gz", "c.txt"],
    ),
    ("a.zip/b.tar.gz", SpecUse::Any, &["a.zip", "b.tar.gz"]),
    (
        "a.zip/b.tar.gz",
        SpecUse::Folder,
        &["a.zip", "b.tar.gz", ""],
    ),
    ("dir.zip/y", SpecUse::Any, &[]),
    ("dir.zip", SpecUse::Folder, &[]),
    ("fake.zip/x", SpecUse::Any, &["fake.zip", "x"]),
    ("fake.zip", SpecUse::Any, &[]),
    ("fake.zip", SpecUse::Folder, &["fake.zip", ""]),
    ("plain.txt/x", SpecUse::Any, &[]),
    ("up.ZIP/x", SpecUse::Any, &["up.ZIP", "x"]),
    ("a.zip/INNER/x", SpecUse::Any, &["a.zip", "INNER/x"]),
    ("none.zip/x", SpecUse::Any, &[]),
    ("a.zip/lnk/x", SpecUse::Any, &["a.zip", "lnk/x"]),
    ("x.tar.gz/y", SpecUse::Any, &["x.tar.gz", "y"]),
    // The folder use of a plain member keeps the archive closed.
    ("a.zip/inner", SpecUse::Folder, &["a.zip", "inner"]),
    ("fake.zip/b.zip/x", SpecUse::Any, &["fake.zip", "b.zip/x"]),
];

/// The spec a chain names.
fn chained(file: Spec, chain: &[&str]) -> Spec {
    chain
        .iter()
        .skip(1)
        .fold(file, |spec, member| Spec::archive_member(spec, *member))
}

fn check_rows(root: &str, sep: &str, style: PathStyle, ctx: &SpecContext) {
    let fake = Fake::new(root, sep);
    for (input, use_, chain) in ROWS {
        let text = format!("{root}{sep}{}", input.replace('/', sep));
        let spec = Spec::parse(&text, ctx).unwrap();
        let got = resolve_with(&fake, spec.clone(), *use_, style);
        let want = if chain.is_empty() {
            spec
        } else {
            let file = Spec::parse(&format!("{root}{sep}{}", chain[0]), ctx).unwrap();
            chained(file, chain)
        };
        assert_eq!(got, want, "{style:?} {input} {use_:?}");
    }
}

#[test]
fn every_row_in_unix_style() {
    let ctx = SpecContext::absolute_only(PathStyle::Unix);
    check_rows("/w", "/", PathStyle::Unix, &ctx);
}

#[test]
fn every_row_in_windows_style() {
    let ctx = SpecContext::absolute_only(PathStyle::Windows);
    check_rows(r"C:\w", r"\", PathStyle::Windows, &ctx);
    check_rows(r"\\srv\share", r"\", PathStyle::Windows, &ctx);
}

/// Rows 24–26: the canonical forms.
#[test]
fn windows_rows_keep_their_text() {
    let ctx = SpecContext::absolute_only(PathStyle::Windows);
    let fake = Fake::new(r"C:\w", r"\");
    let resolve = |text: &str, use_| {
        let spec = Spec::parse(text, &ctx).unwrap();
        resolve_with(&fake, spec, use_, PathStyle::Windows)
    };
    assert_eq!(
        resolve(r"C:\w\a.zip\inner\x", SpecUse::Any).canonical(),
        r"C:\w\a.zip/inner/x"
    );
    assert_eq!(
        resolve(r"C:\w\a.zip\", SpecUse::Folder).canonical(),
        r"C:\w\a.zip/"
    );
    let unc = Fake::new(r"\\srv\share", r"\");
    let spec = Spec::parse(r"\\srv\share\a.zip\x", &ctx).unwrap();
    assert_eq!(
        resolve_with(&unc, spec, SpecUse::Any, PathStyle::Windows).canonical(),
        r"\\srv\share\a.zip/x"
    );
}

/// Row 19 and row 27.
#[test]
fn dots_and_other_specs() {
    let ctx = SpecContext::absolute_only(PathStyle::Unix);
    let fake = Fake::new("/w", "/");
    for text in ["/w/a.zip/../plain.txt", "/w/a.zip/x/../y", "/w/a.zip/.."] {
        let spec = Spec::parse(text, &ctx).unwrap();
        assert!(matches!(spec, Spec::Local(_)), "{text}");
        let got = resolve_with(&fake, spec.clone(), SpecUse::Any, PathStyle::Unix);
        assert_eq!(got, spec, "{text}: `..` never enters");
    }
    for text in ["-", "clipboard://", "ftp://h/a.zip/x"] {
        let spec = Spec::parse(text, &ctx).unwrap();
        for use_ in [SpecUse::Any, SpecUse::Folder] {
            let got = resolve_with(&fake, spec.clone(), use_, PathStyle::Unix);
            assert_eq!(got, spec, "{text}");
        }
    }
}

/// Rows 4, 6 and 28 do not open the archive; row 7 opens the outer one only.
#[test]
fn what_is_opened() {
    let ctx = SpecContext::absolute_only(PathStyle::Unix);
    for text in ["/w/a.zip/inner/x", "/w/a.zip/missing", "/w/x.tar.gz/y"] {
        let fake = Fake::new("/w", "/");
        let spec = Spec::parse(text, &ctx).unwrap();
        resolve_with(&fake, spec, SpecUse::Any, PathStyle::Unix);
        assert!(fake.opened.borrow().is_empty(), "{text}");
    }
    let fake = Fake::new("/w", "/");
    let spec = Spec::parse("/w/a.zip/b.tar.gz/c.txt", &ctx).unwrap();
    resolve_with(&fake, spec, SpecUse::Any, PathStyle::Unix);
    assert_eq!(*fake.opened.borrow(), ["/w/a.zip"]);
}

// ---- on real back-ends ------------------------------------------------------------------------------------------

/// `a.zip` of the rows: `inner/x`, `x`, a nested `b.tar.gz` and a link member.
fn a_zip() -> Vec<u8> {
    use common::archives::RawEntry;
    let inner = tar_bytes(&vec![("c.txt".to_owned(), b"c".to_vec())]);
    let nested = compress(ArchiveFormat::TarGz, &inner);
    archives::raw_zip(&[
        RawEntry::new(b"inner/x", b"member"),
        RawEntry::new(b"x", b"top"),
        RawEntry::new(b"b.tar.gz", &nested),
        RawEntry::new(b"lnk", b"inner").unix(0o120_777),
    ])
}

/// The same files on any `Vfs`, as `(input without the root, use, chain)` through `resolve_spec`.
fn check_back_end(vfs: &dyn Vfs, parse: &dyn Fn(&str) -> Spec) {
    let never = CancelToken::never();
    for (input, use_, chain) in ROWS {
        let got = resolve_spec(vfs, parse(input), *use_, &never).unwrap();
        let want = if chain.is_empty() {
            parse(input)
        } else {
            chained(parse(chain[0]), chain)
        };
        assert_eq!(got, want, "{input} {use_:?}");
    }
}

#[test]
fn rows_on_memory() {
    let memory = on_memory(ArchiveLimits::default());
    for (name, bytes) in [
        ("a.zip", a_zip()),
        ("fake.zip", b"this is text".to_vec()),
        ("plain.txt", b"text".to_vec()),
        ("up.ZIP", zip_bytes(&vec![("x".to_owned(), b"1".to_vec())])),
        (
            "x.tar.gz",
            compress(
                ArchiveFormat::TarGz,
                &tar_bytes(&vec![("y".to_owned(), b"1".to_vec())]),
            ),
        ),
    ] {
        memory.put(name, bytes);
    }
    memory
        .mem
        .insert_file(&host("/dir.zip/y"), b"y".to_vec(), MemMeta::default())
        .unwrap();
    memory
        .mem
        .insert_symlink(&host("/link.zip"), "a.zip", MemMeta::default())
        .unwrap();
    check_back_end(&*memory.vfs, &|input| mspec(&format!("/{input}")));

    // Row 21: a link on disk is followed.
    let never = CancelToken::never();
    let got = resolve_spec(&*memory.vfs, mspec("/link.zip/x"), SpecUse::Any, &never).unwrap();
    assert_eq!(got, Spec::archive_member(mspec("/link.zip"), "x"));
}

#[test]
fn rows_on_disk() {
    let disk = on_disk(ArchiveLimits::default());
    disk.put("a.zip", &a_zip());
    disk.put("fake.zip", b"this is text");
    disk.put("plain.txt", b"text");
    disk.put("up.ZIP", &zip_bytes(&vec![("x".to_owned(), b"1".to_vec())]));
    disk.put(
        "x.tar.gz",
        &compress(
            ArchiveFormat::TarGz,
            &tar_bytes(&vec![("y".to_owned(), b"1".to_vec())]),
        ),
    );
    std::fs::create_dir(disk.dir.join("dir.zip")).unwrap();
    disk.put("dir.zip/y", b"y");
    let ctx = SpecContext::absolute_only(PathStyle::HOST);
    let root = disk.dir.clone();
    check_back_end(&*disk.vfs, &|input: &str| {
        let path: PathBuf = input.split('/').fold(root.clone(), |p, name| p.join(name));
        Spec::parse(path.to_str().unwrap(), &ctx).unwrap()
    });
}

#[test]
fn an_empty_mask_does_not_exist_for_resolution() {
    let memory = on_memory(ArchiveLimits::default());
    let mut config = archives::config(memory.dir.path(), ArchiveLimits::default());
    config.masks = archives::masks_with(|f| (f == ArchiveFormat::Zip).then_some(""));
    memory.vfs.set_config(config);
    memory.put("a.zip", a_zip());
    let spec = mspec("/a.zip/x");
    let got = resolve_spec(
        &*memory.vfs,
        spec.clone(),
        SpecUse::Any,
        &CancelToken::never(),
    );
    assert_eq!(got.unwrap(), spec);
}

/// Row 23.
#[test]
fn nesting_past_the_limit_is_member_text() {
    let limits = ArchiveLimits {
        nesting_max: 2,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let mut bytes = zip_bytes(&vec![("x".to_owned(), b"1".to_vec())]);
    for _ in 0..3 {
        bytes = zip_bytes(&vec![("a.zip".to_owned(), bytes)]);
    }
    memory.put("a.zip", bytes);
    let got = resolve_spec(
        &*memory.vfs,
        mspec("/a.zip/a.zip/a.zip/a.zip/x"),
        SpecUse::Any,
        &CancelToken::never(),
    )
    .unwrap();
    assert_eq!(got.archive_depth(), 3, "{got}");
    assert!(got.canonical().ends_with("a.zip/x"));
    let error = memory.vfs.open_read(&got).map(drop).unwrap_err();
    assert!(
        matches!(
            error,
            hikasa_vfs::VfsError::ArchiveLimit {
                limit: "nesting",
                ..
            }
        ),
        "{error}"
    );
}

/// Call counts: at most one `stat` per component, and rows 4, 6 and 28 open nothing, even when the file cannot be
/// opened.
#[test]
fn call_counts() {
    let memory = on_memory(ArchiveLimits::default());
    memory.put("a.zip", a_zip());
    memory.put("x.tar.gz", b"not read".to_vec());
    memory.mem.inject(hikasa_vfs::Fault {
        path: host("/a.zip"),
        op: hikasa_vfs::VfsOp::Open,
        kind: hikasa_core::ErrorKind::Io,
    });
    let never = CancelToken::never();
    for (input, components) in [
        ("/a.zip/inner/x", 3),
        ("/a.zip/missing", 2),
        ("/x.tar.gz/y", 2),
    ] {
        let before = memory.mem.stat_calls();
        let spec = resolve_spec(&*memory.vfs, mspec(input), SpecUse::Any, &never).unwrap();
        assert!(matches!(spec, Spec::ArchiveMember { .. }), "{input}");
        let stats = memory.mem.stat_calls() - before;
        assert!(stats <= components, "{input}: {stats} stats");
    }
    assert_eq!(memory.vfs.counters().index_passes, 0);
}

#[test]
fn the_same_specs_from_eight_threads() {
    let memory = on_memory(ArchiveLimits::default());
    memory.put("a.zip", a_zip());
    memory.put("plain.txt", b"text".to_vec());
    let inputs = [
        "/a.zip/inner/x",
        "/a.zip/b.tar.gz/c.txt",
        "/a.zip/b.tar.gz",
        "/plain.txt/x",
        "/none/y",
    ];
    let specs: Vec<(Spec, SpecUse)> = (0..1000)
        .map(|i| {
            let use_ = if i % 2 == 0 {
                SpecUse::Any
            } else {
                SpecUse::Folder
            };
            (mspec(inputs[i % inputs.len()]), use_)
        })
        .collect();
    let answers = |order: usize| -> Vec<(usize, Spec)> {
        let never = CancelToken::never();
        let mut out: Vec<(usize, Spec)> = (0..specs.len())
            .map(|i| (i * (order * 2 + 1) % specs.len(), i))
            .map(|(at, _)| {
                let (spec, use_) = &specs[at];
                (
                    at,
                    resolve_spec(&*memory.vfs, spec.clone(), *use_, &never).unwrap(),
                )
            })
            .collect();
        out.sort_by_key(|(at, _)| *at);
        out
    };
    let want = answers(0);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (1..=8)
            .map(|order| {
                let answers = &answers;
                scope.spawn(move || answers(order))
            })
            .collect();
        for handle in handles {
            let got = handle.join().unwrap();
            assert_eq!(got.len(), want.len());
            for ((at, answer), (_, expected)) in got.iter().zip(&want) {
                assert_eq!(answer, &want[*at].1);
                let _ = expected;
            }
        }
    });
}

#[test]
fn resolve_rs_never_reads_the_os() {
    let source = include_str!("../src/archive/resolve.rs");
    assert!(!source.contains("std::fs"));
    assert!(!source.contains("std::env"));
}

#[test]
fn a_cancelled_prepare_cancels_resolution() {
    let memory = on_memory(ArchiveLimits::default());
    memory.put("a.zip", a_zip());
    let cancel = CancelToken::new();
    cancel.cancel();
    let got = resolve_spec(
        &*memory.vfs,
        mspec("/a.zip/b.tar.gz/c.txt"),
        SpecUse::Any,
        &cancel,
    );
    assert!(got.is_err());
}

/// P-ARCH-1: generated nested specs over a generated tree round-trip through `canonical`, `Spec::parse` and
/// `resolve_spec`; `use = Folder` for a root, else `Any`.
fn arb_names() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,6}", 1..4)
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/archive_resolve.txt"))]

    #[test]
    fn p_arch_1_round_trip(outer in arb_names(), inner in arb_names(), root in any::<bool>()) {
        let memory = on_memory(ArchiveLimits::default());
        let inner_path = inner.join("/");
        let tgz = compress(
            ArchiveFormat::TarGz,
            &tar_bytes(&vec![(inner_path.clone(), b"deep".to_vec())]),
        );
        let outer_path = format!("{}/n.tar.gz", outer.join("/"));
        let file = memory.put("a.zip", zip_bytes(&vec![(outer_path.clone(), tgz)]));
        let nested = Spec::archive_member(file, outer_path);
        let (spec, use_) = if root {
            (Spec::archive_member(nested, ""), SpecUse::Folder)
        } else {
            (Spec::archive_member(nested, inner_path), SpecUse::Any)
        };
        let ctx = SpecContext::absolute_only(PathStyle::HOST);
        let parsed = Spec::parse(&spec.canonical(), &ctx).unwrap();
        prop_assert!(matches!(parsed, Spec::Local(_)), "parse never yields a member");
        let back = resolve_spec(&*memory.vfs, parsed, use_, &CancelToken::never()).unwrap();
        prop_assert_eq!(back, spec);
    }
}
