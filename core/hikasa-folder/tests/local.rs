//! LocalVfs integration (§6.2.6): real modification times through `filetime`, Unix permissions, a symlink loop,
//! non-UTF-8 names, and the D-45 prefilter on a real file system.

mod common;

use std::path::Path;

use common::engines;
use hikasa_core::{CancelToken, CompareVerdict, Ctl, LocalPath, NullProgress, Spec};
use hikasa_folder::{
    CompareState, FolderComparison, FolderEnv, FolderRules, FolderRulesPatch, NodeId, NullObserver,
    compare_folders,
};
use hikasa_vfs::{LocalOptions, LocalVfs};

fn spec(p: &Path) -> Spec {
    Spec::Local(LocalPath::new(p.to_path_buf()).unwrap())
}

fn compare(root: &Path, rules: &FolderRules, threads: usize) -> FolderComparison {
    let vfs = LocalVfs::new(LocalOptions::default());
    let engines = engines(1 << 20);
    let now = common::default_now();
    let cancel = CancelToken::never();
    let progress = NullProgress;
    let pool = common::pool(threads);
    let env = FolderEnv {
        vfs: &vfs,
        content: &engines,
        pool: pool.as_ref(),
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 30,
    };
    let cmp = compare_folders(&spec(&root.join("l")), &spec(&root.join("r")), rules, &env).unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

fn child(cmp: &FolderComparison, parent: NodeId, name: &str) -> NodeId {
    let read = cmp.read();
    read.children(parent)
        .find(|&id| {
            read.node(id)
                .unwrap()
                .representative()
                .is_some_and(|e| e.name == name)
        })
        .unwrap_or_else(|| panic!("no child {name}"))
}

fn write(path: &Path, content: &[u8], mtime: i64) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
    filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(mtime, 0)).unwrap();
}

#[test]
fn real_mtimes_sizes_and_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let t = 1_790_000_000;
    write(&root.join("l/same.txt"), b"same", t);
    write(&root.join("r/same.txt"), b"same", t + 1);
    write(&root.join("l/newer.txt"), b"abc", t + 3_600);
    write(&root.join("r/newer.txt"), b"abc", t);
    write(&root.join("l/sub/size.txt"), b"abc", t);
    write(&root.join("r/sub/size.txt"), b"abcd", t);
    write(&root.join("l/only.txt"), b"o", t);
    let cmp = compare(root, &FolderRules::default(), 4);
    let root_id = NodeId::ROOT;
    let read = cmp.read();
    let state = |name: &str| read.node(child(&cmp, root_id, name)).unwrap().state;
    assert_eq!(state("same.txt"), CompareState::Same);
    assert_eq!(state("newer.txt"), CompareState::LeftNewer);
    assert_eq!(state("only.txt"), CompareState::LeftOrphan);
    assert_eq!(state("sub"), CompareState::Different);
    assert_eq!(cmp.verdict(false), CompareVerdict::Differences);
    // With a binary content test the newer pair becomes Same (override, D-48).
    drop(read);
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            content_method: Some(hikasa_core::ContentMethod::Binary),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let cmp = compare(root, &rules, 0);
    let read = cmp.read();
    let newer = child(&cmp, NodeId::ROOT, "newer.txt");
    assert_eq!(read.node(newer).unwrap().state, CompareState::Same);
    assert_eq!(
        read.spec(newer, hikasa_core::Side::Left),
        Some(spec(&root.join("l").join("newer.txt")))
    );
    let sub = child(&cmp, NodeId::ROOT, "sub");
    let size = child(&cmp, sub, "size.txt");
    assert_eq!(read.path(size).as_str(), "sub/size.txt");
    assert_eq!(
        read.spec(size, hikasa_core::Side::Right),
        Some(spec(&root.join("r").join("sub").join("size.txt")))
    );
    assert_eq!(read.totals(sub).right.bytes, 4);
}

#[cfg(unix)]
#[test]
fn unix_permissions_and_symlink_loop() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("l/p.txt"), b"p", 1_790_000_000);
    write(&root.join("r/p.txt"), b"p", 1_790_000_000);
    std::fs::set_permissions(root.join("l/p.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(root.join("r/p.txt"), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::create_dir_all(root.join("l/dir")).unwrap();
    std::fs::create_dir_all(root.join("r/dir/up")).unwrap();
    std::os::unix::fs::symlink("..", root.join("l/dir/up")).unwrap();
    // A link to itself: the OS reports ELOOP when it is followed.
    std::os::unix::fs::symlink("self", root.join("l/self")).unwrap();
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            unix: Some(hikasa_folder::UnixCriteria {
                permissions: true,
                owner: true,
                group: true,
            }),
            follow_symlinks: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let cmp = compare(root, &rules, 2);
    let read = cmp.read();
    let p = read.node(child(&cmp, NodeId::ROOT, "p.txt")).unwrap();
    assert_eq!(p.state, CompareState::Different);
    assert!(p.diffs.contains(hikasa_folder::DiffFlags::PERMISSIONS));
    assert!(!p.diffs.contains(hikasa_folder::DiffFlags::OWNER));
    let dir = child(&cmp, NodeId::ROOT, "dir");
    let up = read.node(child(&cmp, dir, "up")).unwrap();
    assert_eq!(up.scan, hikasa_folder::ScanStatus::Failed);
    assert!(
        up.flags.contains(
            hikasa_folder::NodeFlags::SYMLINK_LOOP | hikasa_folder::NodeFlags::LEFT_ERROR
        )
    );
    assert_eq!(up.state, CompareState::Error);
    let selfloop = read.node(child(&cmp, NodeId::ROOT, "self")).unwrap();
    // The OS reports the cycle as ELOOP: an Error node with SYMLINK_LOOP (D-33), like a link to an ancestor.
    assert!(
        selfloop.flags.contains(
            hikasa_folder::NodeFlags::LEFT_ERROR | hikasa_folder::NodeFlags::SYMLINK_LOOP
        )
    );
    assert_eq!(selfloop.state, CompareState::Error);
    assert_eq!(cmp.verdict(false), CompareVerdict::Differences);
    assert!(
        read.warnings()
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::SymlinkLoop)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let bad = OsStr::from_bytes(b"bad\xff.txt");
    let other = OsStr::from_bytes(b"bad\xfe.txt");
    for side in ["l", "r"] {
        std::fs::create_dir_all(root.join(side)).unwrap();
        write(&root.join(side).join(bad), b"x", 1_790_000_000);
    }
    write(&root.join("l").join(other), b"y", 1_790_000_000);
    let cmp = compare(root, &FolderRules::default(), 0);
    let read = cmp.read();
    let ids: Vec<NodeId> = read.children(NodeId::ROOT).collect();
    assert_eq!(ids.len(), 2, "two different raw names are two nodes");
    let views: Vec<_> = ids.iter().map(|&id| read.node(id).unwrap()).collect();
    assert!(views.iter().all(|v| v.left.unwrap().name_lossy));
    let pair = ids
        .iter()
        .copied()
        .find(|&id| read.node(id).unwrap().right.is_some())
        .unwrap();
    assert_eq!(read.node(pair).unwrap().state, CompareState::Same);
    assert_eq!(
        read.spec(pair, hikasa_core::Side::Left),
        Some(spec(&root.join("l").join(bad)))
    );
    let orphan = ids.iter().copied().find(|&id| id != pair).unwrap();
    assert_eq!(read.node(orphan).unwrap().state, CompareState::LeftOrphan);
    assert_eq!(
        read.spec(orphan, hikasa_core::Side::Left),
        Some(spec(&root.join("l").join(other)))
    );
    assert!(read.spec(orphan, hikasa_core::Side::Right).is_none());
}

/// §2.5.1: case-insensitive alignment folds the exact OS name, never its lossy text. `a\xff` and `a\xfe` (both
/// shown as `a\u{FFFD}`) stay two orphans; `B\xff` and `b\xff` pair, with NAME_CASE when filename case is compared.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_under_case_insensitive_alignment() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for side in ["l", "r"] {
        std::fs::create_dir_all(root.join(side)).unwrap();
    }
    write(
        &root.join("l").join(OsStr::from_bytes(b"a\xff")),
        b"left",
        1_790_000_000,
    );
    write(
        &root.join("r").join(OsStr::from_bytes(b"a\xfe")),
        b"right!",
        1_790_000_000,
    );
    write(
        &root.join("l").join(OsStr::from_bytes(b"B\xff")),
        b"same",
        1_790_000_000,
    );
    write(
        &root.join("r").join(OsStr::from_bytes(b"b\xff")),
        b"same",
        1_790_000_000,
    );
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            case: Some(hikasa_folder::CaseMode::Insensitive),
            filename_case: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    for threads in [0, 4] {
        let cmp = compare(root, &rules, threads);
        let read = cmp.read();
        let nodes: Vec<_> = read
            .children(NodeId::ROOT)
            .map(|id| read.node(id).unwrap())
            .collect();
        assert_eq!(nodes.len(), 3, "{nodes:#?}");
        let pairs: Vec<_> = nodes
            .iter()
            .filter(|n| n.left.is_some() && n.right.is_some())
            .collect();
        assert_eq!(
            pairs.len(),
            1,
            "only B\\xff and b\\xff differ in case alone"
        );
        let pair = pairs[0];
        assert_eq!(pair.state, CompareState::Different);
        assert_eq!(pair.diffs, hikasa_folder::DiffFlags::NAME_CASE);
        let orphans: Vec<CompareState> = nodes
            .iter()
            .filter(|n| n.left.is_none() || n.right.is_none())
            .map(|n| n.state)
            .collect();
        assert_eq!(orphans.len(), 2);
        assert!(orphans.contains(&CompareState::LeftOrphan));
        assert!(orphans.contains(&CompareState::RightOrphan));
    }
}

#[test]
fn prefilter_on_the_local_file_system() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for side in ["l", "r"] {
        write(&root.join(side).join("keep.c"), b"k", 1_790_000_000);
        write(&root.join(side).join("drop.o"), b"d", 1_790_000_000);
        write(&root.join(side).join("build/out.c"), b"o", 1_790_000_000);
    }
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some(r"*.c;-build\".into()),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let cmp = compare(root, &rules, 0);
    let read = cmp.read();
    let dropped = read.node(child(&cmp, NodeId::ROOT, "drop.o")).unwrap();
    assert_eq!(dropped.state, CompareState::Filtered);
    assert!(dropped.left.unwrap().stat_skipped && dropped.right.unwrap().stat_skipped);
    assert_eq!(dropped.left.unwrap().size, 0);
    let build = read.node(child(&cmp, NodeId::ROOT, "build")).unwrap();
    assert_eq!(build.state, CompareState::Filtered);
    assert_eq!(
        read.children(build.id).len(),
        0,
        "an excluded folder is not listed"
    );
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);
}
