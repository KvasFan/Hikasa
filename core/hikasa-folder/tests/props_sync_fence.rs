//! M2 W6c acceptance A6.28 (D-166): a mirror never leaves the bases. Random trees on `MemoryVfs` with links to a folder
//! outside, links the comparison follows (FC-013), opened archives with hostile member names, filters and overrides: after a
//! mirror either way, everything outside the two bases is what it was, byte for byte. A disk variant puts a real link to a
//! folder outside the base and follows it.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::{config_with, default_now, engines, host, mspec, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, Spec};
use hikasa_folder::ops::{ConfirmMode, ConfirmPolicy, Confirmation, OpSettings, ReadOnlySides};
use hikasa_folder::sync::{
    OrphanReading, SyncAction, SyncOptions, SyncPreset, SyncRequest, plan_sync, sync_folder_rules,
};
use hikasa_folder::{FolderError, FolderModel, FolderRules, FolderRulesPatch, compare_into};
use hikasa_vfs::ops::{self, ItemStatus};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, EntryKind, MemMeta,
    MemoryVfs, Vfs,
};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[136..147].copy_from_slice(b"15000000000");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

/// Member names an archive must not let out: up the tree, absolute, through a `..` in the middle.
const HOSTILE: [&str; 5] = [
    "../../out/f.txt",
    "/out/abs.txt",
    "a/../../../out/x",
    "ok.txt",
    "d/inner.txt",
];

/// What an entry of a generated base is.
#[derive(Debug, Clone)]
enum Gen {
    File(u8),
    Dir(Vec<(u8, Gen)>),
    /// A link to the outside folder (absolute).
    LinkOutDir,
    /// A link to a file outside (relative, `../`).
    LinkOutFile,
    /// A tar whose members have hostile names.
    Tar(Vec<u8>),
}

const NAMES: [&str; 6] = ["a", "b.txt", "c", "d.log", "e.tar", "f"];

fn entries() -> impl Strategy<Value = Vec<(u8, Gen)>> {
    let leaf = prop_oneof![
        4 => (0u8..3).prop_map(Gen::File),
        1 => Just(Gen::LinkOutDir),
        1 => Just(Gen::LinkOutFile),
        1 => proptest::collection::vec(0u8..5, 1..4).prop_map(Gen::Tar),
    ];
    let tree = leaf.prop_recursive(3, 20, 4, |inner| {
        proptest::collection::vec((0u8..6, inner), 0..4).prop_map(Gen::Dir)
    });
    proptest::collection::vec((0u8..6, tree), 0..6)
}

fn insert(vfs: &MemoryVfs, dir: &str, entries: &[(u8, Gen)], salt: u8) {
    let mut taken = std::collections::BTreeSet::new();
    for (name, item) in entries {
        if !taken.insert(*name) {
            continue;
        }
        let path = format!("{dir}/{}", NAMES[usize::from(*name)]);
        let secs = 1_000_000 * (1 + i64::from(salt));
        match item {
            Gen::File(content) => {
                let data = format!("{path}{}", "x".repeat(usize::from(*content + salt)));
                vfs.insert_file(&host(&path), data.into_bytes(), meta(secs))
                    .unwrap();
            }
            Gen::Dir(children) => {
                vfs.insert_dir(&host(&path), meta(500)).unwrap();
                insert(vfs, &path, children, salt);
            }
            Gen::LinkOutDir => {
                vfs.insert_symlink(&host(&path), &host("/out/d"), meta(secs))
                    .unwrap();
            }
            Gen::LinkOutFile => {
                let up = "../".repeat(path.matches('/').count() - 1);
                vfs.insert_symlink(&host(&path), &format!("{up}out/f.txt"), meta(secs))
                    .unwrap();
            }
            Gen::Tar(members) => {
                let named: Vec<(&str, &[u8])> = members
                    .iter()
                    .map(|m| (HOSTILE[usize::from(*m)], b"member data".as_slice()))
                    .collect();
                vfs.insert_file(&host(&path), tar_of(&named), meta(secs))
                    .unwrap();
            }
        }
    }
}

/// Everything outside the two bases: path, kind, bytes, time.
fn outside(vfs: &MemoryVfs) -> BTreeMap<String, (EntryKind, Vec<u8>, i64)> {
    let l = host("/l");
    let r = host("/r");
    let sep = if cfg!(windows) { '\\' } else { '/' };
    vfs.dump()
        .into_iter()
        .filter(|e| {
            !(e.path == l
                || e.path == r
                || e.path.starts_with(&format!("{l}{sep}"))
                || e.path.starts_with(&format!("{r}{sep}")))
        })
        .map(|e| {
            let bytes = vfs
                .read_back(&e.path)
                .map(|b| b.to_vec())
                .unwrap_or_default();
            (
                e.path.clone(),
                (e.kind, bytes, e.mtime.map_or(0, |t| t.unix_seconds())),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Case {
    left: Vec<(u8, Gen)>,
    right: Vec<(u8, Gen)>,
    mirror_left: bool,
    follow: bool,
    within: bool,
    filtered: bool,
    over: Option<(u8, u8)>,
    recycle: bool,
    create_empty: bool,
    orphans: bool,
}

fn cases() -> impl Strategy<Value = Case> {
    (
        entries(),
        entries(),
        (
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            proptest::bool::weighted(0.3),
        ),
        (
            proptest::option::weighted(0.3, (0u8..6, 0u8..5)),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                left,
                right,
                (mirror_left, follow, within, filtered),
                (over, recycle, create_empty, orphans),
            )| Case {
                left,
                right,
                mirror_left,
                follow,
                within,
                filtered,
                over,
                recycle,
                create_empty,
                orphans,
            },
        )
}

#[test]
fn a6_28_a_mirror_never_writes_outside_the_bases() {
    let config = config_with("tests/proptest-regressions/props_sync_fence.txt", 256);
    let total = config.cases;
    let mut runner = TestRunner::new(config);
    let ran = AtomicU32::new(0);
    let seen = AtomicU32::new(0);
    runner
        .run(&cases(), |case| {
            seen.fetch_add(1, Ordering::Relaxed);
            let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
            for dir in ["/l", "/r", "/out", "/out/d", "/out/d/sub"] {
                mem.insert_dir(&host(dir), meta(500)).unwrap();
            }
            for (path, bytes) in [
                ("/out/f.txt", "outside file"),
                ("/out/d/a", "outside a"),
                ("/out/d/b.txt", "outside b"),
                ("/out/d/sub/c", "outside c"),
                ("/out/d/e.tar", "not a tar"),
            ] {
                mem.insert_file(&host(path), bytes.as_bytes().to_vec(), meta(3_000_000))
                    .unwrap();
            }
            insert(&mem, "/l", &case.left, 0);
            insert(&mem, "/r", &case.right, 1);
            let cache = tempfile::tempdir().unwrap();
            let masks = ArchiveMasks::new([(ArchiveFormat::Tar, "*.tar".to_owned())]).unwrap();
            let config = ArchiveConfig {
                masks,
                limits: ArchiveLimits::default(),
                clock: Arc::new(default_now),
                cache_root: cache.path().to_path_buf(),
            };
            let vfs = ArchiveVfs::new(mem.clone() as Arc<dyn Vfs>, config);
            let before = outside(&mem);
            let mut options = SyncOptions {
                preset: if case.mirror_left {
                    SyncPreset::MirrorLeft
                } else {
                    SyncPreset::MirrorRight
                },
                within_archives: case.within,
                recycle: case.recycle,
                create_empty: case.create_empty,
                orphans: if case.orphans {
                    OrphanReading::Deleted
                } else {
                    OrphanReading::Added
                },
                ..SyncOptions::default()
            };
            if let Some((name, action)) = case.over {
                options.overrides.insert(
                    NAMES[usize::from(name)].to_owned(),
                    SyncAction::ALL[usize::from(action)],
                );
            }
            let mut rules = FolderRules::default();
            rules.handling.follow_symlinks = case.follow;
            if case.filtered {
                rules
                    .apply(&FolderRulesPatch {
                        name_filter: Some("*.txt;-c\\".to_owned()),
                        ..FolderRulesPatch::default()
                    })
                    .unwrap();
            }
            let rules = sync_folder_rules(&rules, &options).unwrap();
            let engines = engines(1 << 30);
            let cmp = with_env(&vfs, &engines, &CancelToken::new(), |env| {
                let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
                compare_into(&model, env).unwrap()
            });
            let request = SyncRequest {
                options: &options,
                just_selection: None,
                visible: None,
                include_hidden: false,
                iu: false,
                read_only: ReadOnlySides::default(),
                allow_empty_source: true,
                settings: OpSettings::default(),
            };
            let plan = match plan_sync(&cmp.read(), &vfs, &request, &Ctl::detached()) {
                Ok(plan) => plan,
                Err(FolderError::EmptySource { .. }) => return Ok(()),
                Err(e) => return Err(TestCaseError::fail(e.to_string())),
            };
            prop_assert_eq!(&outside(&mem), &before, "a plan wrote outside");
            // Every target and every source a plan removes lies in a base.
            for item in &plan.op.plan.items {
                let in_bases = |spec: &Spec| {
                    let text = spec.outermost_file().unwrap_or(spec).canonical();
                    text.starts_with(&host("/l")) || text.starts_with(&host("/r"))
                };
                prop_assert!(
                    in_bases(&item.target),
                    "{:?} {}",
                    item.action,
                    item.target.canonical()
                );
            }
            let op = Confirmation::new(&plan.op, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
                .finish(plan.op.clone())
                .unwrap();
            let report = ops::execute_with(&vfs, &vfs, op.plan, &op.exec, &Ctl::detached());
            if report.results.iter().any(|r| r.status == ItemStatus::Done) {
                ran.fetch_add(1, Ordering::Relaxed);
            }
            let after = outside(&mem);
            if after != before {
                let changed: Vec<_> = after
                    .iter()
                    .filter(|(k, v)| before.get(*k) != Some(v))
                    .map(|(k, _)| k.clone())
                    .chain(before.keys().filter(|k| !after.contains_key(*k)).cloned())
                    .collect();
                let items: Vec<String> = report
                    .plan
                    .items
                    .iter()
                    .zip(&report.results)
                    .map(|(i, r)| format!("{:?} {} {:?}", i.action, i.target.canonical(), r.status))
                    .collect();
                prop_assert!(false, "outside changed: {changed:?}\nitems {items:#?}");
            }
            // The bin is outside too: nothing of the outside folder went there.
            for trashed in mem.trashed() {
                prop_assert!(
                    trashed.starts_with(&host("/l")) || trashed.starts_with(&host("/r")),
                    "{trashed} went to the bin"
                );
            }
            Ok(())
        })
        .unwrap();
    let seen = seen.load(Ordering::Relaxed).max(1);
    let ran = ran.load(Ordering::Relaxed);
    eprintln!("A6.28: {ran} of {seen} cases ran an item");
    assert!(
        ran * 2 >= seen.min(total),
        "only {ran} of {seen} cases ran an item"
    );
}

/// A link to a folder outside the base, on disk, followed: a mirror onto that side writes nothing through it, and a mirror
/// that deletes it removes the link only.
#[test]
fn a6_28_disk_a_followed_link_out_of_the_base_keeps_the_outside() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (l, r, out) = (root.join("l"), root.join("r"), root.join("out"));
    for folder in [&l, &r, &out, &out.join("sub"), &l.join("d")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(out.join("keep.txt"), b"outside").unwrap();
    std::fs::write(out.join("sub/deep.txt"), b"outside deep").unwrap();
    std::fs::write(l.join("d/keep.txt"), b"left version").unwrap();
    std::fs::write(l.join("top.txt"), b"top").unwrap();
    if !folder_link(&out, &r.join("d")) || !folder_link(&out, &r.join("gone")) {
        eprintln!("skipped: this system does not let the test make a folder link");
        return;
    }
    let vfs = hikasa_vfs::LocalVfs::new(hikasa_vfs::LocalOptions::default());
    let spec = |path: &std::path::Path| {
        Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    for preset in [SyncPreset::MirrorRight, SyncPreset::MirrorLeft] {
        let options = SyncOptions {
            preset,
            create_empty: true,
            ..SyncOptions::default()
        };
        let mut rules = FolderRules::default();
        rules.handling.follow_symlinks = true;
        let rules = sync_folder_rules(&rules, &options).unwrap();
        let engines = engines(1 << 30);
        let cmp = with_env(&vfs, &engines, &CancelToken::new(), |env| {
            let model = FolderModel::new(spec(&l), spec(&r), rules.clone()).unwrap();
            compare_into(&model, env).unwrap()
        });
        let request = SyncRequest {
            options: &options,
            just_selection: None,
            visible: None,
            include_hidden: false,
            iu: false,
            read_only: ReadOnlySides::default(),
            allow_empty_source: false,
            settings: OpSettings::default(),
        };
        let plan = plan_sync(&cmp.read(), &vfs, &request, &Ctl::detached()).unwrap();
        let op = Confirmation::new(&plan.op, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
            .finish(plan.op.clone())
            .unwrap();
        ops::execute_with(&vfs, &vfs, op.plan, &op.exec, &Ctl::detached());
        assert_eq!(
            std::fs::read(out.join("keep.txt")).unwrap(),
            b"outside",
            "{preset:?}"
        );
        assert_eq!(
            std::fs::read(out.join("sub/deep.txt")).unwrap(),
            b"outside deep",
            "{preset:?}"
        );
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2, "{preset:?}");
    }
    // Mirror to Right removed the link `gone` (the left has none), never what it leads to.
    assert!(std::fs::symlink_metadata(r.join("gone")).is_err());
}

#[cfg(unix)]
fn folder_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn folder_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn folder_link(_target: &std::path::Path, _link: &std::path::Path) -> bool {
    false
}
