//! M2 W5b acceptance A5.17 and P-ARCH-2 of A5.16 (D-149): a rebuild that meets any fault leaves the archive as it was.
//!
//! Property over small trees × format × fault: a failed read of the container, a failed write, a failed `commit`, a
//! refused `open_write`, cancellation at a random point, a nested rebuild failing at each level, also with a level in
//! the chain that nothing changes. `MemoryVfs::dump()` of the whole tree is unchanged, so no temporary sibling remains
//! on this back-end (a real disk: `no_temporary_sibling_stays`), every done item of the root is `failed` (`not_run` when
//! cancelled), the summary matches, and a later retry on the same tree succeeds. The moves: local → archive,
//! archive → archive, archive → local.
//!
//! P-ARCH-2: a random script of operations on `MemoryVfs` and on an archive of the same tree ends in equal listings and
//! contents, read back by this back-end, by a new one and by the plain crates. The names run past every field of a tar
//! header, whose archives have ustar headers, and the archive can lie below a level nothing changes.
//!
//! 48 cases per property unless `PROPTEST_CASES` says otherwise: trees stay small, so memory and time stay bounded.

mod common;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

use common::archives::{
    CancellingBase, OnMemory, Seen, TreeData, bytes_of, config, name_of, on_memory, read_plain,
    ustar_of, walk,
};
use common::{ctl, host, mspec};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, Spec};
use hikasa_vfs::ops::{
    self, CopyOptions, ItemStatus, MoveOptions, OpReport, OverwritePolicy, Plan, Preserve,
    RemoveOptions,
};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, Fault, FollowLinks, ListOptions, MemDump, MemMeta,
    MemoryVfs, Vfs, VfsOp, WriteOptions,
};
use proptest::prelude::*;
use proptest::test_runner::Config;

fn config_of() -> Config {
    let mut config = common::config("tests/proptest-regressions/archive_faults.txt");
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 48;
    }
    config
}

const WRITABLE: [ArchiveFormat; 6] = [
    ArchiveFormat::Zip,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarBz2,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
];

fn format_strategy() -> impl Strategy<Value = ArchiveFormat> {
    (0usize..WRITABLE.len()).prop_map(|at| WRITABLE[at])
}

/// Small trees: up to 12 members, some in folders.
fn tree_strategy() -> impl Strategy<Value = TreeData> {
    prop::collection::btree_map(
        prop_oneof!["[a-e]", "d[0-2]/[a-e]", "d[0-2]/s/[a-e]"],
        prop::collection::vec(any::<u8>(), 0..2000),
        1..12,
    )
    .prop_map(|map| {
        let mut files: BTreeMap<String, Vec<u8>> = map;
        // A path that is also a folder would be refused: the file goes.
        let names: Vec<String> = files.keys().cloned().collect();
        for name in names {
            if files
                .keys()
                .any(|other| other.starts_with(&format!("{name}/")))
            {
                files.remove(&name);
            }
        }
        files.into_iter().collect()
    })
}

#[derive(Debug, Clone, Copy)]
enum Faulty {
    /// `open_read` of the container gives this many bytes, then fails.
    ReadAfter(u64),
    /// The writer of the container takes this many bytes, then fails.
    WriteAfter(u64),
    /// `open_write` of the container fails.
    Open,
    /// `commit` fails.
    Commit,
    /// The job is cancelled after this many bytes of output.
    CancelAfter(u64),
}

fn fault_strategy() -> impl Strategy<Value = Faulty> {
    prop_oneof![
        (0u64..4000).prop_map(Faulty::ReadAfter),
        (0u64..4000).prop_map(Faulty::WriteAfter),
        Just(Faulty::Open),
        Just(Faulty::Commit),
        (0u64..4000).prop_map(Faulty::CancelAfter),
    ]
}

/// A memory tree and a back-end whose rebuild cancels `token` at a byte, for [`Faulty::CancelAfter`].
fn on_cancelling(token: &CancelToken, after: u64) -> OnMemory {
    let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let base = CancellingBase {
        mem: Arc::clone(&mem),
        token: token.clone(),
        after,
    };
    let vfs = Arc::new(ArchiveVfs::new(
        Arc::new(base) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    ));
    OnMemory { mem, vfs, dir }
}

/// The back-end `fault` needs.
fn back_end(fault: Faulty, token: &CancelToken) -> OnMemory {
    match fault {
        Faulty::CancelAfter(after) => on_cancelling(token, after),
        _ => on_memory(ArchiveLimits::default()),
    }
}

fn dump_of(memory: &OnMemory) -> Vec<MemDump> {
    memory.mem.dump()
}

fn arm(memory: &OnMemory, path: &str, fault: Faulty) {
    let host_path = host(path);
    match fault {
        Faulty::ReadAfter(bytes) => {
            memory
                .mem
                .inject_read_error_after(&host_path, bytes, ErrorKind::Io);
        }
        Faulty::WriteAfter(bytes) => {
            memory
                .mem
                .inject_write_error_after(&host_path, bytes, ErrorKind::Io);
        }
        Faulty::Open => memory.mem.inject(Fault {
            path: host_path,
            op: VfsOp::Write,
            kind: ErrorKind::PermissionDenied,
        }),
        Faulty::Commit => memory.mem.inject(Fault {
            path: host_path,
            op: VfsOp::Commit,
            kind: ErrorKind::Io,
        }),
        Faulty::CancelAfter(_) => {}
    }
}

/// A copy of two files and a folder into `root`.
fn into_archive_plan(vfs: &dyn Vfs, root: &Spec, opts: &CopyOptions) -> Plan {
    let pairs = [(
        mspec("/src/tree"),
        Spec::archive_member(root.clone(), "incoming"),
    )];
    ops::plan_copy(vfs, vfs, &pairs, opts, &Ctl::detached()).unwrap()
}

fn put_sources(memory: &OnMemory) {
    let meta = MemMeta::default();
    memory
        .mem
        .insert_file(&host("/src/tree/one"), &b"first source"[..], meta.clone())
        .unwrap();
    memory
        .mem
        .insert_file(&host("/src/tree/sub/two"), &vec![9u8; 3000][..], meta)
        .unwrap();
}

fn assert_summary(report: &OpReport) {
    let count = |status| report.results.iter().filter(|r| r.status == status).count() as u64;
    assert_eq!(report.summary.done, count(ItemStatus::Done));
    assert_eq!(report.summary.failed, count(ItemStatus::Failed));
    assert_eq!(report.summary.skipped, count(ItemStatus::Skipped));
    assert_eq!(report.summary.planned, report.results.len() as u64);
}

proptest! {
    #![proptest_config(config_of())]

    /// Any fault during a rebuild leaves the archive and its folder as they were, and a retry succeeds.
    #[test]
    fn a_fault_leaves_the_archive_untouched(
        tree in tree_strategy(),
        format in format_strategy(),
        fault in fault_strategy(),
    ) {
        let cancel = CancelToken::new();
        let memory = back_end(fault, &cancel);
        let name = name_of(format);
        let file = memory.put(name, bytes_of(format, &tree));
        put_sources(&memory);
        let vfs = &*memory.vfs;
        let opts = CopyOptions { recursive: true, preserve: Preserve::default(), ..CopyOptions::default() };
        // Read the index before the fault: a real reader has one too.
        let root = Spec::archive_member(file.clone(), "");
        vfs.list(&root, &ListOptions::default(), &CancelToken::never()).unwrap();
        let before = dump_of(&memory);
        let plan = into_archive_plan(vfs, &file, &opts);
        arm(&memory, &format!("/{name}"), fault);
        let report = ops::execute(vfs, vfs, plan, &ctl(&cancel));
        assert_summary(&report);
        if report.ok() {
            // A cancel or a read fault that came too late to matter: the rebuild went through.
            prop_assert!(matches!(fault, Faulty::CancelAfter(_) | Faulty::ReadAfter(_) | Faulty::WriteAfter(_)));
        } else {
            prop_assert_eq!(dump_of(&memory), before.clone(), "{:?}", fault);
            let done = report.results.iter().filter(|r| r.status == ItemStatus::Done).count();
            prop_assert_eq!(done, 0, "{:?}", report.results);
            if report.cancelled {
                prop_assert!(report.results.iter().all(|r| r.status == ItemStatus::NotRun));
            } else {
                prop_assert!(report.results.iter().all(|r| r.status == ItemStatus::Failed));
            }
            prop_assert_eq!(vfs.cache_used(), 0, "staged files are released");
            memory.mem.clear_faults();
            let plan = into_archive_plan(vfs, &file, &opts);
            let retry = ops::execute(vfs, vfs, plan, &Ctl::detached());
            prop_assert!(retry.ok(), "{:?}", retry.results);
        }
        memory.mem.clear_faults();
        let incoming = Spec::archive_member(file.clone(), "incoming/one");
        prop_assert_eq!(common::read_to_vec(vfs.open_read(&incoming).unwrap()), b"first source");
        let seen = walk(vfs, &root);
        let plain = read_plain(format, &memory.mem.read_back(&host(&format!("/{name}"))).unwrap());
        prop_assert_eq!(seen, plain);
    }

    /// The moves keep every copy a failed rebuild needs.
    #[test]
    fn a_failed_move_keeps_its_sources(
        tree in tree_strategy(),
        format in format_strategy(),
        fault in prop_oneof![Just(Faulty::Open), Just(Faulty::Commit), (0u64..8).prop_map(Faulty::WriteAfter)],
    ) {
        // Local → archive: a failing rebuild keeps all sources.
        let memory = on_memory(ArchiveLimits::default());
        let name = name_of(format);
        let file = memory.put(name, bytes_of(format, &tree));
        put_sources(&memory);
        let vfs = &*memory.vfs;
        let before = dump_of(&memory);
        let pairs = [(mspec("/src/tree"), Spec::archive_member(file.clone(), "moved"))];
        let plan = ops::plan_move(vfs, vfs, &pairs, &MoveOptions::default(), &Ctl::detached()).unwrap();
        arm(&memory, &format!("/{name}"), fault);
        let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
        assert_summary(&report);
        prop_assert!(!report.ok());
        prop_assert_eq!(dump_of(&memory), before);

        // Archive → archive: a failing target keeps the source archive unchanged.
        let memory = on_memory(ArchiveLimits::default());
        let source = memory.put("source.tar", bytes_of(ArchiveFormat::Tar, &tree));
        let target = memory.put(name, bytes_of(format, &tree));
        let vfs = &*memory.vfs;
        let (from, _) = tree.first().unwrap();
        let pairs = [(Spec::archive_member(source.clone(), from.as_str()), Spec::archive_member(target.clone(), "moved"))];
        let before = dump_of(&memory);
        let plan = ops::plan_move(vfs, vfs, &pairs, &MoveOptions::default(), &Ctl::detached()).unwrap();
        arm(&memory, &format!("/{name}"), fault);
        let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
        assert_summary(&report);
        prop_assert!(!report.ok());
        prop_assert_eq!(dump_of(&memory), before);

        // Archive → local: a failing source rebuild keeps both copies.
        let memory = on_memory(ArchiveLimits::default());
        let source = memory.put(name, bytes_of(format, &tree));
        memory.mem.insert_dir(&host("/out"), MemMeta::default()).unwrap();
        let vfs = &*memory.vfs;
        let spec = Spec::archive_member(source.clone(), from.as_str());
        let pairs = [(spec.clone(), mspec("/out/moved"))];
        let plan = ops::plan_move(vfs, vfs, &pairs, &MoveOptions::default(), &Ctl::detached()).unwrap();
        arm(&memory, &format!("/{name}"), fault);
        let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
        assert_summary(&report);
        prop_assert!(!report.ok());
        prop_assert!(vfs.stat(&spec, FollowLinks::No).is_ok(), "the source stays");
        prop_assert!(memory.mem.read_back(&host("/out/moved")).is_some(), "and the copy");
    }

    /// A nested rebuild that fails at any level leaves the outermost file as it was.
    #[test]
    fn a_nested_rebuild_fails_as_a_whole(
        tree in tree_strategy(),
        inner in format_strategy(),
        outer in format_strategy(),
        fault in fault_strategy(),
        untouched in any::<bool>(),
    ) {
        let cancel = CancelToken::new();
        let memory = back_end(fault, &cancel);
        let mut nested: TreeData = vec![
            (name_of(inner).to_owned(), bytes_of(inner, &tree)),
            ("side".to_owned(), b"side".to_vec()),
        ];
        let name = name_of(outer);
        // A level between them that the operation only passes through.
        if untouched {
            nested = vec![("mid.tar".to_owned(), bytes_of(ArchiveFormat::Tar, &nested))];
        }
        let file = memory.put(name, bytes_of(outer, &nested));
        put_sources(&memory);
        let vfs = &*memory.vfs;
        let around = if untouched {
            Spec::archive_member(file, "mid.tar")
        } else {
            file
        };
        let chain = Spec::archive_member(around, name_of(inner));
        let opts = CopyOptions { recursive: true, preserve: Preserve::default(), ..CopyOptions::default() };
        let before = dump_of(&memory);
        let plan = into_archive_plan(vfs, &chain, &opts);
        arm(&memory, &format!("/{name}"), fault);
        let report = ops::execute(vfs, vfs, plan, &ctl(&cancel));
        assert_summary(&report);
        if !report.ok() {
            prop_assert_eq!(dump_of(&memory), before);
            prop_assert_eq!(vfs.cache_used(), 0);
        }
    }
}

// ---- P-ARCH-2 ---------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Step {
    Write(u8, Vec<u8>),
    Mkdir(u8),
    Remove(u8),
    RemoveTree(u8),
    Rename(u8, u8),
    Copy(u8, u8),
    Move(u8, u8),
    Link(u8),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (any::<u8>(), prop::collection::vec(any::<u8>(), 0..300)).prop_map(|(a, b)| Step::Write(a, b)),
        2 => any::<u8>().prop_map(Step::Mkdir),
        2 => any::<u8>().prop_map(Step::Remove),
        1 => any::<u8>().prop_map(Step::RemoveTree),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Step::Rename(a, b)),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Step::Copy(a, b)),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Step::Move(a, b)),
        1 => any::<u8>().prop_map(Step::Link),
    ]
}

/// A folder and a name for a path that fits a tar header only split, or not at all.
const LONG_DIR: &str =
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd/";
const LONG_NAME: &str = "nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn";

/// A path from a small set, so steps meet each other.
fn path_of(selector: u8) -> String {
    const DIRS: [&str; 5] = ["", "p/", "q/", "p/r/", LONG_DIR];
    const NAMES: [&str; 5] = ["a", "b", "p", "q", LONG_NAME];
    let dir = DIRS[usize::from(selector >> 4) % DIRS.len()];
    let name = NAMES[usize::from(selector & 15) % NAMES.len()];
    format!("{dir}{name}")
}

/// The outcome of a step: what kind of failure a back-end gave, so two back-ends must agree on that too. A path through
/// a link is the one difference: a disk follows the link and finds nothing, an archive never follows one (rule 5).
fn outcome(report: &OpReport) -> Vec<(String, Option<ErrorKind>)> {
    let kind = |kind| match kind {
        ErrorKind::NotADirectory => ErrorKind::NotFound,
        other => other,
    };
    report
        .results
        .iter()
        .map(|r| {
            let status = format!("{:?}", r.status);
            (status, r.error.as_ref().map(|e| kind(e.kind)))
        })
        .collect()
}

fn run(step: &Step, vfs: &dyn Vfs, at: &dyn Fn(&str) -> Spec) -> Vec<(String, Option<ErrorKind>)> {
    let ctl = Ctl::detached();
    let copy = CopyOptions {
        recursive: true,
        overwrite: OverwritePolicy::Always,
        preserve: Preserve::NONE,
        ..CopyOptions::default()
    };
    let moved = MoveOptions {
        overwrite: OverwritePolicy::Fail,
        ..MoveOptions::default()
    };
    let remove = |recursive| RemoveOptions {
        recursive,
        trash: false,
    };
    match step {
        Step::Write(path, bytes) => {
            let options = WriteOptions {
                overwrite: true,
                replace_link: true,
                ..WriteOptions::default()
            };
            let done = vfs
                .open_write(&at(&path_of(*path)), &options)
                .and_then(|mut writer| {
                    writer.write_all(bytes).unwrap();
                    writer.commit()
                });
            vec![(format!("{}", done.is_ok()), None)]
        }
        Step::Mkdir(path) => {
            let done = vfs.create_dir(&at(&path_of(*path)), false);
            vec![(format!("{}", done.is_ok()), None)]
        }
        Step::Remove(path) => {
            let plan = ops::plan_remove(vfs, &[at(&path_of(*path))], &remove(false), &ctl).unwrap();
            outcome(&ops::execute(vfs, vfs, plan, &ctl))
        }
        Step::RemoveTree(path) => {
            let plan = ops::plan_remove(vfs, &[at(&path_of(*path))], &remove(true), &ctl).unwrap();
            outcome(&ops::execute(vfs, vfs, plan, &ctl))
        }
        Step::Rename(from, to) => {
            let done = vfs.rename(&at(&path_of(*from)), &at(&path_of(*to)), false);
            vec![(format!("{}", done.is_ok()), None)]
        }
        Step::Copy(from, to) => {
            let pairs = [(at(&path_of(*from)), at(&path_of(*to)))];
            let plan = ops::plan_copy(vfs, vfs, &pairs, &copy, &ctl).unwrap();
            outcome(&ops::execute(vfs, vfs, plan, &ctl))
        }
        Step::Move(from, to) => {
            let pairs = [(at(&path_of(*from)), at(&path_of(*to)))];
            let plan = ops::plan_move(vfs, vfs, &pairs, &moved, &ctl).unwrap();
            outcome(&ops::execute(vfs, vfs, plan, &ctl))
        }
        Step::Link(path) => {
            let spec = at(&path_of(*path));
            let done = vfs.create_symlink(&spec, std::ffi::OsStr::new("target"), false);
            vec![(format!("{}", done.is_ok()), None)]
        }
    }
}

/// The model: a folder of `MemoryVfs`, as a tree.
fn model_view(vfs: &dyn Vfs, root: &Spec) -> BTreeMap<String, Seen> {
    walk(vfs, root)
}

proptest! {
    #![proptest_config(config_of())]

    /// P-ARCH-2. `MemoryVfs` is the model: an archive of the same tree goes through the same steps, one batch per
    /// step outside `ops`, and ends the same, for this back-end, a new one and the plain crates.
    #[test]
    fn random_operations_match_the_model(
        steps in prop::collection::vec(step_strategy(), 1..24),
        format in format_strategy(),
        batched in any::<bool>(),
        chained in any::<bool>(),
    ) {
        let start: TreeData = vec![
            ("a".into(), b"first".to_vec()),
            ("p/b".into(), b"second".to_vec()),
            ("p/r/a".into(), b"third".to_vec()),
            ("q/".into(), Vec::new()),
            (format!("{LONG_DIR}{LONG_NAME}"), b"fourth".to_vec()),
            (format!("{LONG_DIR}a"), b"fifth".to_vec()),
        ];
        let model = MemoryVfs::new(CaseSensitivity::Sensitive);
        for (path, content) in &start {
            let full = host(&format!("/m/{path}"));
            if path.ends_with('/') {
                model.insert_dir(full.trim_end_matches(['/', '\\']), MemMeta::default()).unwrap();
            } else {
                model.insert_file(&full, content.clone(), MemMeta::default()).unwrap();
            }
        }
        let memory = on_memory(ArchiveLimits::default());
        let name = name_of(format);
        let bytes = ustar_of(format, &start);
        // Below a zip and a tar that only pass the change on.
        let file = if chained {
            let middle: TreeData = vec![(name.to_owned(), bytes)];
            let outer: TreeData = vec![("mid.tar".to_owned(), bytes_of(ArchiveFormat::Tar, &middle))];
            let outermost = memory.put("outer.zip", bytes_of(ArchiveFormat::Zip, &outer));
            Spec::archive_member(Spec::archive_member(outermost, "mid.tar"), name)
        } else {
            memory.put(name, bytes)
        };
        let vfs = &*memory.vfs;
        let in_model = |path: &str| mspec(&format!("/m/{path}"));
        let in_archive = |path: &str| Spec::archive_member(file.clone(), path);
        if batched {
            vfs.begin(&file).unwrap();
        }
        for step in &steps {
            if batched && !matches!(step, Step::Write(..) | Step::Mkdir(_) | Step::Rename(..) | Step::Link(_)) {
                continue;
            }
            let want = run(step, &model, &in_model);
            let got = run(step, vfs, &in_archive);
            prop_assert_eq!(&got, &want, "{:?}", step);
        }
        if batched {
            vfs.finish(&file, hikasa_vfs::Finish::Commit).unwrap();
        }
        let want = model_view(&model, &mspec("/m"));
        prop_assert_eq!(walk(vfs, &in_archive("")), want.clone());
        let dir = tempfile::tempdir().unwrap();
        let fresh = ArchiveVfs::new(
            Arc::clone(&memory.mem) as Arc<dyn Vfs>,
            config(dir.path(), ArchiveLimits::default()),
        );
        prop_assert_eq!(walk(&fresh, &in_archive("")), want.clone());
        let inner = |bytes: &[u8], format, member: &str| match read_plain(format, bytes).remove(member) {
            Some(Seen::File(bytes)) => bytes,
            other => panic!("{other:?}"),
        };
        let bytes = if chained {
            let outer = memory.mem.read_back(&host("/outer.zip")).unwrap();
            let middle = inner(&outer, ArchiveFormat::Zip, "mid.tar");
            inner(&middle, ArchiveFormat::Tar, name)
        } else {
            memory.mem.read_back(&host(&format!("/{name}"))).unwrap().to_vec()
        };
        let plain = read_plain(format, &bytes);
        prop_assert_eq!(plain, want);
    }
}
