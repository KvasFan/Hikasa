//! M2 W6b acceptance A6.16 (D-178) and A6.17 (D-179): conflicts by the table of §1.6.11, the question machine, and the recycle
//! bin by the table of §1.6.12. The CLI's own paths are in `cli_folder_op.rs`.

mod common;

use std::sync::Arc;

use common::{compared, default_now, engines, host, mspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, NullProgress, RelPath, Side, Spec,
    WarningCode,
};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, Conflicts, Declined, FolderOp, OpPlan, OpScope,
    OpSettings, QuestionKind, ReadOnlySides, Recycle, Reply, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{
    ArchiveHandling, FolderComparison, FolderEnv, FolderModel, FolderRules, NullObserver,
    compare_into,
};
use hikasa_vfs::ops::{self, Action, ItemStatus, OpReport, OverwritePolicy, SkipReason};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, Attrs, Capabilities,
    DosAttrs, EntryKind, Fault, MemMeta, MemoryVfs, UnixMeta, Vfs, VfsOp,
};

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn dos(secs: i64, attrs: DosAttrs) -> MemMeta {
    MemMeta {
        attrs: Some(Attrs::Dos { attrs }),
        ..meta(secs)
    }
}

fn unix(secs: i64, mode: u32) -> MemMeta {
    MemMeta {
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: 0,
                gid: 0,
            },
        }),
        ..meta(secs)
    }
}

fn memory() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    vfs
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], with: MemMeta) {
    vfs.insert_file(&host(path), content.to_vec(), with)
        .unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn plan(vfs: &dyn Vfs, cmp: &FolderComparison, rows: &[(&str, SideSet)], op: &FolderOp) -> OpPlan {
    let read = cmp.read();
    let mut selection = Selection::default();
    for (row, sides) in rows {
        selection.insert(read.node_at(&RelPath::parse(row).unwrap()).unwrap(), *sides);
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    plan_op(
        &read,
        vfs,
        &scope,
        op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap()
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

const L: SideSet = SideSet::LEFT;
const R: SideSet = SideSet::RIGHT;
const COPY: FolderOp = FolderOp::Copy {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};
const MOVE: FolderOp = FolderOp::Move {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};

fn delete(sides: SideSet, recycle: Recycle) -> FolderOp {
    FolderOp::Delete { sides, recycle }
}

fn no_bin() -> Recycle {
    Recycle::default()
}

fn bin() -> Recycle {
    Recycle {
        use_bin: true,
        fallback: false,
    }
}

/// What the plan's first item conflicts with.
fn conflicts(out: &OpPlan) -> Conflicts {
    out.meta[0].conflicts
}

fn replace_case(target: MemMeta) -> (MemoryVfs, FolderComparison) {
    let vfs = memory();
    file(&vfs, "/l/x", b"new", meta(10));
    file(&vfs, "/r/x", b"old", target);
    let cmp = compared(&vfs, &FolderRules::default());
    (vfs, cmp)
}

#[test]
fn a6_16_read_only_as_a_dos_bit_and_a_unix_mode() {
    for (target, want) in [
        (dos(10, DosAttrs::READONLY), Conflicts::READ_ONLY),
        (dos(10, DosAttrs::empty()), Conflicts::empty()),
        (unix(10, 0o444), Conflicts::READ_ONLY),
        (unix(10, 0o644), Conflicts::empty()),
        (unix(10, 0o200), Conflicts::empty()),
    ] {
        let (vfs, cmp) = replace_case(target);
        assert_eq!(conflicts(&plan(&vfs, &cmp, &[("x", L)], &COPY)), want);
        assert_eq!(
            conflicts(&plan(&vfs, &cmp, &[("x", R)], &delete(R, no_bin()))),
            want
        );
    }
}

#[test]
fn a6_16_system() {
    let (vfs, cmp) = replace_case(dos(10, DosAttrs::SYSTEM));
    assert_eq!(
        conflicts(&plan(&vfs, &cmp, &[("x", L)], &COPY)),
        Conflicts::SYSTEM
    );
    assert_eq!(
        conflicts(&plan(&vfs, &cmp, &[("x", R)], &delete(R, no_bin()))),
        Conflicts::SYSTEM
    );
}

#[test]
fn a6_16_overwrite_newer_by_the_rows_relation() {
    let (vfs, cmp) = replace_case(meta(20));
    assert_eq!(
        conflicts(&plan(&vfs, &cmp, &[("x", L)], &COPY)),
        Conflicts::OVERWRITE_NEWER
    );
    let (vfs, cmp) = replace_case(meta(5));
    assert_eq!(
        conflicts(&plan(&vfs, &cmp, &[("x", L)], &COPY)),
        Conflicts::empty()
    );
}

#[test]
fn a6_16_replace_during_move() {
    let (vfs, cmp) = replace_case(meta(5));
    assert_eq!(
        conflicts(&plan(&vfs, &cmp, &[("x", L)], &MOVE)),
        Conflicts::REPLACE_DURING_MOVE
    );
}

/// Three items in conflict, in plan order: `a` read-only and system, `b` read-only and newer, `c` read-only.
fn three() -> (MemoryVfs, OpPlan) {
    let vfs = memory();
    for name in ["a", "b", "c"] {
        file(&vfs, &format!("/l/{name}"), b"new", meta(10));
    }
    file(
        &vfs,
        "/r/a",
        b"old",
        dos(10, DosAttrs::READONLY | DosAttrs::SYSTEM),
    );
    file(&vfs, "/r/b", b"old", dos(20, DosAttrs::READONLY));
    file(&vfs, "/r/c", b"old", dos(10, DosAttrs::READONLY));
    file(&vfs, "/l/clean", b"new", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(
        &vfs,
        &cmp,
        &[("a", L), ("b", L), ("c", L), ("clean", L)],
        &COPY,
    );
    (vfs, out)
}

fn kinds(machine: &mut Confirmation, reply: Reply) -> Vec<(QuestionKind, Option<u32>)> {
    let mut seen = Vec::new();
    while let Some(question) = machine.next() {
        seen.push((question.kind, question.item));
        machine.reply(reply);
    }
    seen
}

#[test]
fn a6_16_order_is_the_operation_then_items_then_class_by_class() {
    let (_, out) = three();
    let mut machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(machine.next().unwrap().remaining, 6);
    assert!(machine.next().unwrap().text.contains("copy"));
    assert_eq!(
        kinds(&mut machine, Reply::Yes),
        [
            (QuestionKind::Operation, None),
            (QuestionKind::ReadOnly, Some(0)),
            (QuestionKind::System, Some(0)),
            (QuestionKind::ReadOnly, Some(1)),
            (QuestionKind::OverwriteNewer, Some(1)),
            (QuestionKind::ReadOnly, Some(2)),
        ]
    );
}

#[test]
fn a6_16_yes_to_all_is_per_class() {
    let (_, out) = three();
    let mut machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    machine.reply(Reply::Yes);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::ReadOnly);
    machine.reply(Reply::YesToAll);
    // Yes to read-only is not yes to system or newer.
    assert_eq!(machine.next().unwrap().kind, QuestionKind::System);
    machine.reply(Reply::Yes);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::OverwriteNewer);
    machine.reply(Reply::No);
    assert!(machine.next().is_none());
    let done = machine.finish(out).unwrap();
    assert_eq!(done.plan.items[0].action, Action::Copy);
    assert!(done.plan.items[0].clear_read_only);
    assert_eq!(done.meta[0].answer, Some(true));
    assert_eq!(done.plan.items[1].action, Action::Skip);
    assert_eq!(done.plan.items[1].skip, Some(SkipReason::Declined));
    assert!(!done.plan.items[1].clear_read_only);
    assert_eq!(done.meta[1].answer, Some(false));
    assert!(done.plan.items[2].clear_read_only);
    assert_eq!(done.meta[3].answer, None);
}

#[test]
fn a6_16_no_to_all_is_per_class_too() {
    let (_, out) = three();
    let mut machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    machine.reply(Reply::Yes);
    machine.reply(Reply::NoToAll);
    assert!(machine.next().is_none());
    let done = machine.finish(out).unwrap();
    for at in 0..3 {
        assert_eq!(done.plan.items[at].skip, Some(SkipReason::Declined), "{at}");
    }
    assert_eq!(done.plan.items[3].action, Action::Copy);
}

#[test]
fn a6_16_modes() {
    let (_, out) = three();
    let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::YesToAll);
    assert!(machine.next().is_none());
    let done = machine.finish(out.clone()).unwrap();
    assert!(done.plan.items.iter().all(|item| item.skip.is_none()));
    assert!(
        done.plan
            .items
            .iter()
            .take(3)
            .all(|item| item.clear_read_only)
    );
    assert!(!done.plan.items[3].clear_read_only);
    let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::NoToAll);
    assert!(machine.next().is_none());
    let done = machine.finish(out).unwrap();
    let skipped = done
        .plan
        .items
        .iter()
        .filter(|item| item.skip.is_some())
        .count();
    assert_eq!(skipped, 3);
    assert_eq!(done.plan.items[3].action, Action::Copy);
}

#[test]
fn a6_16_cancel_and_no_to_the_operation_are_declined() {
    let (_, out) = three();
    let mut machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    machine.reply(Reply::No);
    assert!(machine.next().is_none());
    assert_eq!(machine.finish(out.clone()), Err(Declined));
    let mut machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    machine.reply(Reply::Yes);
    machine.reply(Reply::Cancel);
    assert!(machine.next().is_none());
    assert_eq!(machine.finish(out.clone()), Err(Declined));
    // What was not answered does not run.
    let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(machine.finish(out), Err(Declined));
}

#[test]
fn a6_16_a_key_that_is_off_drops_its_question_and_the_item_proceeds() {
    let (_, out) = three();
    let policy = ConfirmPolicy {
        copy: false,
        read_only: false,
        system: false,
        overwrite_newer: false,
        ..ConfirmPolicy::ALL
    };
    let machine = Confirmation::new(&out, &policy, ConfirmMode::Prompt);
    assert!(machine.next().is_none());
    let done = machine.finish(out.clone()).unwrap();
    assert!(done.plan.items.iter().all(|item| item.skip.is_none()));
    assert!(done.plan.items[0].clear_read_only);
    // `no-to-all` as a mode leaves them too: no key asks.
    let machine = Confirmation::new(&out, &policy, ConfirmMode::NoToAll);
    let done = machine.finish(out).unwrap();
    assert!(done.plan.items.iter().all(|item| item.skip.is_none()));
}

#[test]
fn a6_16_the_operation_question_of_each_verb() {
    let vfs = memory();
    file(&vfs, "/l/a", b"1", meta(10));
    file(&vfs, "/r/a", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let rename = FolderOp::Rename {
        sides: L,
        mask: hikasa_folder::ops::RenameMask::Name("b".to_owned()),
    };
    let cases = [
        (COPY, Some(QuestionKind::Operation)),
        (MOVE, Some(QuestionKind::Operation)),
        (delete(L, no_bin()), Some(QuestionKind::Operation)),
        (rename, None),
    ];
    for (op, want) in cases {
        let out = plan(&vfs, &cmp, &[("a", L)], &op);
        let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
        assert_eq!(machine.next().map(|q| q.kind), want, "{op:?}");
    }
    // A plan where nothing runs has nothing to confirm.
    let skip = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Skip,
    };
    let out = plan(&vfs, &cmp, &[("a", L)], &skip);
    let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    assert!(machine.next().is_none());
    // `exchange` asks `confirm_move`.
    let out = plan(&vfs, &cmp, &[("a", L)], &FolderOp::Exchange);
    let policy = ConfirmPolicy {
        move_: false,
        ..ConfirmPolicy::ALL
    };
    assert!(
        Confirmation::new(&out, &policy, ConfirmMode::Prompt)
            .next()
            .is_none()
    );
}

#[test]
fn a6_16_a_read_only_yes_clears_r_before_replace_and_remove() {
    let vfs = memory();
    vfs.set_capabilities(vfs.capabilities(&mspec("/l")).unwrap() | Capabilities::SET_ATTRS);
    file(&vfs, "/l/x", b"new", meta(20));
    file(&vfs, "/r/x", b"old", dos(10, DosAttrs::READONLY));
    file(&vfs, "/r/y", b"old", dos(10, DosAttrs::READONLY));
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("x", L)], &COPY);
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/x").unwrap(), b"new");
    let out = plan(&vfs, &cmp, &[("y", R)], &delete(R, no_bin()));
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(read(&vfs, "/r/y").is_none());
}

/// On disk: a read-only target, `R` on Windows, a mode without write bits elsewhere.
#[test]
fn a6_16_a_read_only_target_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (l, r) = (root.join("l"), root.join("r"));
    std::fs::create_dir(&l).unwrap();
    std::fs::create_dir(&r).unwrap();
    std::fs::write(l.join("x"), b"new").unwrap();
    std::fs::write(r.join("x"), b"old").unwrap();
    std::fs::write(r.join("y"), b"old").unwrap();
    for name in ["x", "y"] {
        let path = r.join(name);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
    }
    let spec = |path: &std::path::Path| {
        Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    let vfs = hikasa_vfs::LocalVfs::new(hikasa_vfs::LocalOptions::default());
    let contents = engines(1 << 20);
    let compare = || {
        common::with_env(&vfs, &contents, &CancelToken::new(), |env| {
            let model = FolderModel::new(spec(&l), spec(&r), FolderRules::default()).unwrap();
            compare_into(&model, env).unwrap()
        })
    };
    let cmp = compare();
    let out = plan(&vfs, &cmp, &[("x", L)], &COPY);
    assert_eq!(out.meta[0].conflicts, Conflicts::READ_ONLY);
    // No keeps it.
    let declined = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::NoToAll)
        .finish(out.clone())
        .unwrap();
    run(&vfs, declined);
    assert_eq!(std::fs::read(r.join("x")).unwrap(), b"old");
    // Yes replaces it, and removes the other.
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(std::fs::read(r.join("x")).unwrap(), b"new");
    let cmp = compare();
    let out = plan(&vfs, &cmp, &[("y", R)], &delete(R, no_bin()));
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(!r.join("y").exists());
}

#[test]
fn a6_16_a_declined_folder_cuts_what_lies_below_it() {
    let vfs = memory();
    file(&vfs, "/l/same/inner", b"1", meta(10));
    file(&vfs, "/r/same", b"2", dos(10, DosAttrs::READONLY));
    let cmp = compared(&vfs, &FolderRules::default());
    let read_tree = cmp.read();
    let folder = read_tree
        .children(read_tree.root())
        .find(|&row| read_tree.node(row).unwrap().is_folder())
        .unwrap();
    let mut selection = Selection::default();
    selection.insert(folder, L);
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let out = plan_op(
        &read_tree,
        &vfs,
        &scope,
        &COPY,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read_tree);
    assert_eq!(out.meta[0].conflicts, Conflicts::READ_ONLY);
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::NoToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    // The answer is why what lies below stays: nothing is left `not_run`, and the plan is a success.
    for result in &report.results {
        assert_eq!(result.status, ItemStatus::Skipped);
        assert_eq!(result.skip, Some(SkipReason::Declined));
    }
    assert!(report.ok());
    assert_eq!(read(&vfs, "/r/same").unwrap(), b"2");
}

// ---- A6.17 ---------------------------------------------------------------------------------------------------

fn actions(out: &OpPlan) -> Vec<(Action, String)> {
    out.plan
        .items
        .iter()
        .map(|item| {
            let name = item.target.canonical().replace('\\', "/");
            (
                item.action,
                name.strip_prefix("C:").unwrap_or(&name).to_owned(),
            )
        })
        .collect()
}

#[test]
fn a6_17_the_table_files_links_special() {
    let vfs = memory();
    file(&vfs, "/l/f", b"1", meta(10));
    vfs.insert_symlink(&host("/l/link"), "f", meta(10)).unwrap();
    vfs.insert_other(&host("/l/pipe"), meta(10)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let rows = [("f", L), ("link", L), ("pipe", L)];
    let out = plan(&vfs, &cmp, &rows, &delete(L, no_bin()));
    assert!(
        actions(&out)
            .iter()
            .all(|(action, _)| *action == Action::Remove)
    );
    let out = plan(&vfs, &cmp, &rows, &delete(L, bin()));
    assert!(
        actions(&out)
            .iter()
            .all(|(action, _)| *action == Action::Trash)
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(vfs.trashed().len(), 3);
}

#[test]
fn a6_17_a_whole_folder_is_one_trash() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", meta(10));
    file(&vfs, "/l/d/sub/y", b"2", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, bin()));
    assert_eq!(actions(&out), [(Action::Trash, "/l/d".to_owned())]);
    assert_eq!(out.plan.items[0].kind, EntryKind::Dir);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(vfs.trashed(), [host("/l/d")]);
}

#[test]
fn a6_17_not_whole_goes_item_by_item_and_the_folder_stays() {
    let vfs = memory();
    file(&vfs, "/l/d/x.txt", b"1", meta(10));
    file(&vfs, "/l/d/keep.bak", b"2", meta(10));
    let mut rules = FolderRules::default();
    rules.filters.names.exclude_files = vec!["*.bak".to_owned()];
    let cmp = compared(&vfs, &rules);
    for (recycle, action) in [(no_bin(), Action::Remove), (bin(), Action::Trash)] {
        let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, recycle));
        assert_eq!(actions(&out), [(action, "/l/d/x.txt".to_owned())]);
    }
    let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, bin()));
    run(&vfs, out);
    assert_eq!(vfs.trashed(), [host("/l/d/x.txt")]);
    assert!(read(&vfs, "/l/d/keep.bak").is_some());
}

/// Tar members on memory: a member has no `TRASH`.
struct World {
    mem: Arc<MemoryVfs>,
    vfs: ArchiveVfs,
    _dir: tempfile::TempDir,
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

fn world() -> World {
    let mem = Arc::new(memory());
    let dir = tempfile::tempdir().unwrap();
    let masks = ArchiveMasks::new([(ArchiveFormat::Tar, "*.tar".to_owned())]).unwrap();
    let config = ArchiveConfig {
        masks,
        limits: ArchiveLimits::default(),
        clock: Arc::new(default_now),
        cache_root: dir.path().to_path_buf(),
    };
    let vfs = ArchiveVfs::new(mem.clone() as Arc<dyn Vfs>, config);
    World {
        mem,
        vfs,
        _dir: dir,
    }
}

impl World {
    fn opened(&self, rules: &FolderRules) -> FolderComparison {
        let contents = engines(1 << 20);
        let now = default_now();
        let cancel = CancelToken::never();
        let progress = NullProgress;
        let env = FolderEnv {
            vfs: &self.vfs,
            content: &contents,
            pool: None,
            ctl: Ctl {
                cancel: &cancel,
                progress: &progress,
            },
            observer: &NullObserver,
            now: &now,
            content_budget: 1 << 30,
            archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
        };
        let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
        compare_into(&model, &env).unwrap()
    }
}

#[test]
fn a6_17_a_member_is_no_recycle_bin_yes_and_no() {
    let world = world();
    world
        .mem
        .insert_file(
            &host("/l/a.tar"),
            tar_of(&[("m.txt", b"1"), ("n.txt", b"2")]),
            meta(10),
        )
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.archives = ArchiveHandling::Always;
    let cmp = world.opened(&rules);
    let out = plan(&world.vfs, &cmp, &[("a.tar/m.txt", L)], &delete(L, bin()));
    assert_eq!(out.plan.items[0].action, Action::Remove);
    assert_eq!(out.meta[0].conflicts, Conflicts::NO_RECYCLE_BIN);
    let machine = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::Operation);
    // Off in the options it is still asked; no drops the item.
    let policy = ConfirmPolicy {
        delete: false,
        ..ConfirmPolicy::ALL
    };
    let mut machine = Confirmation::new(&out, &policy, ConfirmMode::Prompt);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::NoRecycleBin);
    machine.reply(Reply::No);
    let done = machine.finish(out.clone()).unwrap();
    assert_eq!(done.plan.items[0].skip, Some(SkipReason::Declined));
    let mut machine = Confirmation::new(&out, &policy, ConfirmMode::Prompt);
    machine.reply(Reply::Yes);
    let done = machine.finish(out).unwrap();
    let report = run(&world.vfs, done);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(
        world
            .vfs
            .stat(
                &Spec::archive_member(mspec("/l/a.tar"), "m.txt"),
                hikasa_vfs::FollowLinks::No
            )
            .is_err()
    );
    assert_eq!(world.mem.trashed(), Vec::<String>::new());
}

#[test]
fn a6_17_a_failing_trash_without_fallback_fails_with_the_item_in_place() {
    let vfs = memory();
    file(&vfs, "/l/f", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("f", L)], &delete(L, bin()));
    vfs.inject(Fault {
        path: host("/l/f"),
        op: VfsOp::Trash,
        kind: ErrorKind::Unsupported,
    });
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/l/f").unwrap(), b"1");
    assert!(report.warnings.is_empty());
}

#[test]
fn a6_17_with_fallback_the_report_shows_remove_items_and_one_warning() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", meta(10));
    file(&vfs, "/l/d/sub/y", b"2", meta(10));
    file(&vfs, "/l/f", b"3", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let recycle = Recycle {
        use_bin: true,
        fallback: true,
    };
    let out = plan(&vfs, &cmp, &[("d", L), ("f", L)], &delete(L, recycle));
    assert!(out.plan.items.iter().all(|item| item.trash_fallback));
    for path in ["/l/d", "/l/f"] {
        vfs.inject(Fault {
            path: host(path),
            op: VfsOp::Trash,
            kind: ErrorKind::Unsupported,
        });
    }
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let names: Vec<&str> = report
        .plan
        .items
        .iter()
        .map(|item| item.action.as_str())
        .collect();
    assert_eq!(
        names,
        ["remove", "remove_dir", "remove", "remove_dir", "remove"],
    );
    let warned: Vec<_> = report
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::TrashFallback)
        .collect();
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].message.contains('5'), "{warned:?}");
    assert!(read(&vfs, "/l/d/x").is_none());
    assert!(read(&vfs, "/l/f").is_none());
}

/// §1.6.12: the fallback deletes what the plan saw. A file that came since is no item, stays, and keeps its folder.
#[test]
fn a6_17_the_fallback_deletes_only_what_the_plan_saw() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let recycle = Recycle {
        use_bin: true,
        fallback: true,
    };
    let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, recycle));
    assert_eq!(out.plan.items.len(), 1);
    assert_eq!(out.plan.items[0].fallback.len(), 2);
    assert!(out.plan.items[0].fallback.iter().all(|i| i.expect.is_some()));
    file(&vfs, "/l/d/precious", b"new work", meta(20));
    vfs.inject(Fault {
        path: host("/l/d"),
        op: VfsOp::Trash,
        kind: ErrorKind::Unsupported,
    });
    let report = run(&vfs, out);
    assert!(!report.ok(), "{:?}", report.results);
    assert_eq!(report.summary.failed, 1, "{:?}", report.results);
    assert_eq!(read(&vfs, "/l/d/precious").unwrap(), b"new work");
    assert!(read(&vfs, "/l/d/x").is_none());
}

/// A file edited since the plan is no file the plan saw: its stamp keeps it, and its folder.
#[test]
fn a6_17_the_fallback_goes_by_the_stamp() {
    let recycle = Recycle {
        use_bin: true,
        fallback: true,
    };
    let vfs = memory();
    file(&vfs, "/l/f", b"1", meta(10));
    file(&vfs, "/l/d/x", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("d", L), ("f", L)], &delete(L, recycle));
    file(&vfs, "/l/d/x", b"edited", meta(30));
    for path in ["/l/d", "/l/f"] {
        vfs.inject(Fault {
            path: host(path),
            op: VfsOp::Trash,
            kind: ErrorKind::Unsupported,
        });
    }
    let report = run(&vfs, out);
    assert!(!report.ok());
    assert_eq!(read(&vfs, "/l/d/x").unwrap(), b"edited");
    assert!(read(&vfs, "/l/f").is_none());
    // A trash with nothing stamped below it deletes nothing: a plan read back has no fallback to run.
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let mut out = plan(&vfs, &cmp, &[("d", L)], &delete(L, recycle));
    out.plan.items[0].fallback.clear();
    vfs.inject(Fault {
        path: host("/l/d"),
        op: VfsOp::Trash,
        kind: ErrorKind::Unsupported,
    });
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/l/d/x").unwrap(), b"1");
}

/// A folder the model never listed: the listing at planning is all the fallback may take.
#[test]
fn a6_17_the_fallback_of_an_unlisted_folder() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", meta(10));
    let mut rules = FolderRules::default();
    rules.handling.scan_orphan_folders = false;
    let cmp = compared(&vfs, &rules);
    let recycle = Recycle {
        use_bin: true,
        fallback: true,
    };
    let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, recycle));
    assert_eq!(out.plan.items.len(), 1);
    assert_eq!(out.plan.items[0].action, Action::Trash);
    file(&vfs, "/l/d/precious", b"new", meta(20));
    vfs.inject(Fault {
        path: host("/l/d"),
        op: VfsOp::Trash,
        kind: ErrorKind::Unsupported,
    });
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 1, "{:?}", report.results);
    assert_eq!(read(&vfs, "/l/d/precious").unwrap(), b"new");
}

/// One `trash` of a folder asks for what the model knows below it, and the listing of rule 6 knows as much.
#[test]
fn a6_16_one_trash_asks_for_what_lies_below_it() {
    for scanned in [true, false] {
        let vfs = memory();
        file(&vfs, "/l/d/sub/x", b"1", unix(10, 0o444));
        let mut rules = FolderRules::default();
        rules.handling.scan_orphan_folders = scanned;
        let cmp = compared(&vfs, &rules);
        let out = plan(&vfs, &cmp, &[("d", L)], &delete(L, bin()));
        assert_eq!(out.plan.items.len(), 1, "{:?}", out.plan.items);
        assert_eq!(out.plan.items[0].action, Action::Trash);
        assert_eq!(out.meta[0].conflicts, Conflicts::READ_ONLY);
        let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::NoToAll)
            .finish(out)
            .unwrap();
        assert_eq!(run(&vfs, done).results[0].skip, Some(SkipReason::Declined));
        assert_eq!(vfs.trashed(), Vec::<String>::new());
    }
}

#[test]
fn a6_17_a_gone_item_never_falls_back() {
    let vfs = memory();
    file(&vfs, "/l/f", b"1", meta(10));
    let cmp = compared(&vfs, &FolderRules::default());
    let recycle = Recycle {
        use_bin: true,
        fallback: true,
    };
    let mut out = plan(&vfs, &cmp, &[("f", L)], &delete(L, recycle));
    out.plan.items[0].expect = None;
    vfs.remove(&mspec("/l/f")).unwrap();
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(report.plan.items[0].action, Action::Trash);
    assert!(report.warnings.is_empty());
}
