//! M2 W6b acceptance A6.15 (D-177): one test per builder rule 1–17 of §1.6.10, on `MemoryVfs`. The plan never writes, and the
//! plan that a dry run shows is the one that executes.

mod common;

use common::{compared, config_with, engines, host, mspec, with_env};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, RelPath, Side, Spec, Warning,
    WarningCode,
};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, Conflicts, FolderOp, OpPlan, OpScope, OpSettings,
    ReadOnlySides, Recycle, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{
    CaseMode, Change, Cmp, DisplayOptions, FolderComparison, FolderDisplayFilter, FolderError,
    FolderMode, FolderModel, FolderRules, FolderRulesPatch, NodeId, ScanStatus, SizeFilter,
    TreeRead, compare_into,
};
use hikasa_vfs::ops::{
    self, Action, BackupPolicy, BackupStyle, Expect, ItemStamp, ItemStatus, LinkPolicy, OpReport,
    OverwritePolicy, SkipReason,
};
use hikasa_vfs::{
    Attrs, Capabilities, EntryKind, Fault, ListOptions, Listing, MemMeta, MemoryVfs, ReadSeek,
    UnixMeta, Vfs, VfsError, VfsOp,
};
use proptest::prelude::*;

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn bases(case: CaseSensitivity) -> MemoryVfs {
    let vfs = MemoryVfs::new(case);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    vfs
}

fn memory() -> MemoryVfs {
    bases(CaseSensitivity::Sensitive)
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn all_shown() -> Include {
    Include {
        hidden: false,
        display: DisplayOptions {
            filter: FolderDisplayFilter::All,
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
        },
    }
}

fn id(read: &TreeRead<'_>, path: &str) -> NodeId {
    read.node_at(&RelPath::parse(path).unwrap())
        .unwrap_or_else(|| panic!("no row {path}"))
}

fn selection(read: &TreeRead<'_>, rows: &[(&str, SideSet)]) -> Selection {
    let mut sel = Selection::default();
    for (path, sides) in rows {
        sel.insert(id(read, path), *sides);
    }
    sel
}

fn scope(read: &TreeRead<'_>, rows: &[(&str, SideSet)]) -> OpScope {
    OpScope {
        selection: selection(read, rows),
        include: all_shown(),
        iu: false,
        read_only: ReadOnlySides::default(),
    }
}

fn plan_with(
    vfs: &dyn Vfs,
    cmp: &FolderComparison,
    rows: &[(&str, SideSet)],
    op: &FolderOp,
    settings: &OpSettings,
) -> OpPlan {
    let read = cmp.read();
    plan_op(
        &read,
        vfs,
        &scope(&read, rows),
        op,
        settings,
        &Ctl::detached(),
    )
    .unwrap()
}

fn plan(vfs: &dyn Vfs, cmp: &FolderComparison, rows: &[(&str, SideSet)], op: &FolderOp) -> OpPlan {
    plan_with(vfs, cmp, rows, op, &OpSettings::default())
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

/// The path of a spec below `/`, in `/` form, for the tables of a test.
fn short(spec: &Spec) -> String {
    let text = spec.canonical().replace('\\', "/");
    text.strip_prefix("C:").map(str::to_owned).unwrap_or(text)
}

/// `<action> [<source> >] <target> @<depth>[ !<skip or error kind>][ +replaces]`.
fn lines(plan: &OpPlan) -> Vec<String> {
    plan.plan
        .items
        .iter()
        .map(|item| {
            let mut line = item.action.as_str().to_owned();
            if let Some(source) = &item.source {
                line.push(' ');
                line.push_str(&short(source));
                line.push_str(" >");
            }
            line.push(' ');
            line.push_str(&short(&item.target));
            line.push_str(&format!(" @{}", item.depth));
            if let Some(reason) = item.skip {
                line.push_str(&format!(" !{}", reason.as_str()));
            }
            if let Some(error) = &item.error {
                line.push_str(&format!(" !{}", error.kind.as_str()));
            }
            if item.replaces {
                line.push_str(" +replaces");
            }
            line
        })
        .collect()
}

const COPY_RIGHT: FolderOp = FolderOp::Copy {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};
const MOVE_RIGHT: FolderOp = FolderOp::Move {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};
const L: SideSet = SideSet::LEFT;
const R: SideSet = SideSet::RIGHT;

fn delete(sides: SideSet) -> FolderOp {
    FolderOp::Delete {
        sides,
        recycle: Recycle::default(),
    }
}

// ---- rule 1 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule1_sources_are_the_source_side_alone() {
    let vfs = memory();
    file(&vfs, "/l/a", b"1", 10);
    file(&vfs, "/l/b", b"2", 10);
    file(&vfs, "/r/b", b"3", 20);
    let cmp = compared(&vfs, &FolderRules::default());
    // `b` selected on the right alone is no source of a copy to the right.
    let out = plan(&vfs, &cmp, &[("a", L), ("b", R)], &COPY_RIGHT);
    assert_eq!(lines(&out), ["copy /l/a > /r/a @0"]);
    let read = cmp.read();
    let err = plan_op(
        &read,
        &vfs,
        &scope(&read, &[("b", R)]),
        &COPY_RIGHT,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "rules: nothing selected on the left");
    assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::InvalidInput);
}

// ---- rule 2 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule2_a_pair_writes_to_the_other_sides_own_spec() {
    // The extension row: `x.txt` ↔ `x.csv`.
    let vfs = memory();
    file(&vfs, "/l/x.txt", b"left", 20);
    file(&vfs, "/r/x.csv", b"right", 10);
    let mut rules = FolderRules::default();
    rules.alignment.align_extensions = true;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("x.txt", L)], &COPY_RIGHT);
    assert_eq!(lines(&out), ["copy /l/x.txt > /r/x.csv @0 +replaces"]);
}

#[test]
fn a6_15_rule2_normalization_and_case_rows_stay_aligned() {
    // NFC against NFD.
    let vfs = memory();
    file(&vfs, "/l/caf\u{e9}", b"left", 20);
    file(&vfs, "/r/cafe\u{301}", b"right", 10);
    let mut rules = FolderRules::default();
    rules.alignment.align_normalization = true;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("caf\u{e9}", L)], &COPY_RIGHT);
    assert_eq!(out.plan.items.len(), 1);
    assert_eq!(short(&out.plan.items[0].target), "/r/cafe\u{301}");
    // A case row, on a case-insensitive instance.
    let vfs = bases(CaseSensitivity::Insensitive);
    file(&vfs, "/l/A.txt", b"left", 20);
    file(&vfs, "/r/a.TXT", b"right", 10);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Insensitive;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("A.txt", L)], &COPY_RIGHT);
    assert_eq!(short(&out.plan.items[0].target), "/r/a.TXT");
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/a.TXT").unwrap(), b"left");
}

#[test]
fn a6_15_rule2_an_orphan_goes_under_the_other_sides_own_folder() {
    let vfs = bases(CaseSensitivity::Insensitive);
    file(&vfs, "/l/Dir/x", b"1", 10);
    file(&vfs, "/r/dir/y", b"2", 10);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Insensitive;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("Dir/x", L)], &COPY_RIGHT);
    assert_eq!(lines(&out), ["copy /l/Dir/x > /r/dir/x @0"]);
}

/// `EILSEQ` ("illegal byte sequence"): what a file system that keeps names as UTF-8 answers a name that is not.
#[cfg(unix)]
const EILSEQ: i32 = if cfg!(target_os = "linux") {
    84
} else if cfg!(target_vendor = "apple") {
    92
} else {
    86
};

#[cfg(unix)]
#[test]
fn a6_15_rule2_a_name_that_is_not_utf8_survives_on_disk() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (l, r) = (root.join("l"), root.join("r"));
    std::fs::create_dir(&l).unwrap();
    std::fs::create_dir(&r).unwrap();
    let raw = OsStr::from_bytes(b"caf\xe9");
    // APFS and other file systems that keep names as UTF-8 refuse this one (`EILSEQ`); where names are bytes it runs.
    match std::fs::write(l.join(raw), b"x") {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(EILSEQ) => {
            eprintln!("skipped: this file system refuses a name that is not UTF-8 ({e})");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    let spec = |path: &std::path::Path| {
        Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    let vfs = hikasa_vfs::LocalVfs::new(hikasa_vfs::LocalOptions::default());
    let engines = engines(1 << 30);
    let cmp = with_env(&vfs, &engines, &CancelToken::new(), |env| {
        let model = FolderModel::new(spec(&l), spec(&r), FolderRules::default()).unwrap();
        compare_into(&model, env).unwrap()
    });
    let read = cmp.read();
    let row = read.children(read.root()).next().unwrap();
    let mut sel = Selection::default();
    sel.insert(row, L);
    let scope = OpScope {
        selection: sel,
        include: all_shown(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let out = plan_op(
        &read,
        &vfs,
        &scope,
        &COPY_RIGHT,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read);
    let report = run(&vfs, out);
    assert_eq!(report.summary.done, 1, "{:?}", report.results);
    assert_eq!(std::fs::read(r.join(raw)).unwrap(), b"x");
}

#[test]
fn a6_15_rule2_a_raw_name_on_memory_goes_through_child() {
    let vfs = memory();
    file(&vfs, "/l/name with space", b"x", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("name with space", L)], &COPY_RIGHT);
    assert_eq!(short(&out.plan.items[0].target), "/r/name with space");
}

// ---- rule 3 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule3_missing_folders_once_going_down() {
    let vfs = memory();
    file(&vfs, "/l/a/b/c.txt", b"1", 10);
    file(&vfs, "/l/a/b/d.txt", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(
        &vfs,
        &cmp,
        &[("a/b/c.txt", L), ("a/b/d.txt", L)],
        &COPY_RIGHT,
    );
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/a > /r/a @0",
            "create_dir /l/a/b > /r/a/b @1",
            "copy /l/a/b/c.txt > /r/a/b/c.txt @2",
            "copy /l/a/b/d.txt > /r/a/b/d.txt @2",
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0);
    assert_eq!(read(&vfs, "/r/a/b/d.txt").unwrap(), b"2");
}

#[test]
fn a6_15_rule3_a_folder_that_exists_gets_no_item() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/r/a/y", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", L)], &COPY_RIGHT);
    assert_eq!(lines(&out), ["copy /l/a/x > /r/a/x @0"]);
}

#[test]
fn a6_15_rule3_a_created_folder_gets_its_times_after_its_subtree() {
    let vfs = memory();
    vfs.insert_dir(&host("/l/a"), meta(77)).unwrap();
    file(&vfs, "/l/a/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", L)], &COPY_RIGHT);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0);
    let entry = vfs
        .stat(&mspec("/r/a"), hikasa_vfs::FollowLinks::No)
        .unwrap();
    assert_eq!(entry.mtime, FileTime::from_unix(77, 0));
}

// ---- rule 4 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule4_orders() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/l/a/b/y", b"2", 10);
    file(&vfs, "/l/z", b"3", 10);
    vfs.set_device(&host("/r"), 7);
    let cmp = compared(&vfs, &FolderRules::default());
    // Deletes post-order.
    let out = plan(&vfs, &cmp, &[("a", L), ("z", L)], &delete(L));
    assert_eq!(
        lines(&out),
        [
            "remove /l/a/b/y @2",
            "remove_dir /l/a/b @1",
            "remove /l/a/x @1",
            "remove_dir /l/a @0",
            "remove /l/z @0",
        ]
    );
    // A move: pre-order, `remove_dir` after the contents.
    let out = plan(&vfs, &cmp, &[("a", L)], &MOVE_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/a > /r/a @0",
            "create_dir /l/a/b > /r/a/b @1",
            "move /l/a/b/y > /r/a/b/y @2",
            "remove_dir /l/a/b @1",
            "move /l/a/x > /r/a/x @1",
            "remove_dir /l/a @0",
        ]
    );
}

#[test]
fn a6_15_rule4_a_declined_child_blocks_the_parents_remove_dir() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/l/a/y", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let mut out = plan(&vfs, &cmp, &[("a", L)], &delete(L));
    let first = out
        .plan
        .items
        .iter_mut()
        .find(|item| item.action == Action::Remove)
        .unwrap();
    first.action = Action::Skip;
    first.skip = Some(SkipReason::Declined);
    let report = run(&vfs, out);
    let last = report.results.last().unwrap();
    assert_eq!(last.status, ItemStatus::Skipped);
    assert_eq!(last.skip, Some(SkipReason::ChildrenFailed));
    assert!(read(&vfs, "/l/a/x").is_some());
    assert!(read(&vfs, "/l/a/y").is_none());
}

// ---- rule 5 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule5_kinds() {
    let vfs = memory();
    file(&vfs, "/l/f", b"1", 10);
    vfs.insert_symlink(&host("/l/link"), "f", meta(10)).unwrap();
    vfs.insert_other(&host("/l/pipe"), meta(10)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let rows = [("f", L), ("link", L), ("pipe", L)];
    assert_eq!(
        lines(&plan(&vfs, &cmp, &rows, &COPY_RIGHT)),
        [
            "copy /l/f > /r/f @0",
            "link /l/link > /r/link @0",
            "skip /l/pipe > /r/pipe @0 !special_file",
        ]
    );
    let settings = OpSettings {
        links: LinkPolicy::Skip,
        ..OpSettings::default()
    };
    let out = plan_with(&vfs, &cmp, &rows, &COPY_RIGHT, &settings);
    assert_eq!(lines(&out)[1], "skip /l/link > /r/link @0 !symlink");
}

#[test]
fn a6_15_rule5_a_followed_source_reads_through_its_link() {
    let vfs = memory();
    file(&vfs, "/elsewhere/data", b"content", 10);
    file(&vfs, "/elsewhere/dir/inside", b"x", 10);
    vfs.insert_symlink(&host("/l/file"), &host("/elsewhere/data"), meta(10))
        .unwrap();
    vfs.insert_symlink(&host("/l/dir"), &host("/elsewhere/dir"), meta(10))
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.follow_symlinks = true;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("file", L), ("dir", L)], &COPY_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/dir > /r/dir @0",
            "copy /l/dir/inside > /r/dir/inside @1",
            "copy /l/file > /r/file @0",
        ]
    );
    assert!(out.plan.items[0].follow_source);
    assert!(out.plan.items[2].follow_source);
    assert_eq!(
        out.plan.items[2].expect.unwrap().source.unwrap().kind,
        EntryKind::File
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/file").unwrap(), b"content");
    assert_eq!(read(&vfs, "/r/dir/inside").unwrap(), b"x");
}

// ---- rule 6 --------------------------------------------------------------------------------------------------

/// Counts `list` calls.
struct Lists<'a> {
    inner: &'a MemoryVfs,
    seen: std::sync::Mutex<Vec<String>>,
}

impl Vfs for Lists<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(
        &self,
        spec: &Spec,
        follow: hikasa_vfs::FollowLinks,
    ) -> Result<hikasa_vfs::Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.seen.lock().unwrap().push(short(dir));
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &hikasa_vfs::WriteOptions,
    ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &hikasa_vfs::FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
}

#[test]
fn a6_15_rule6_a_folder_not_scanned_is_spliced_and_only_it_is_listed() {
    let vfs = memory();
    file(&vfs, "/l/sub/x", b"1", 10);
    file(&vfs, "/l/sub/deep/y", b"2", 10);
    vfs.insert_dir(&host("/r/sub"), meta(1)).unwrap();
    let mut rules = FolderRules::default();
    rules.handling.auto_scan_subfolders = false;
    let cmp = compared(&vfs, &rules);
    let spy = Lists {
        inner: &vfs,
        seen: std::sync::Mutex::new(Vec::new()),
    };
    {
        let read = cmp.read();
        assert_eq!(
            read.node(id(&read, "sub")).unwrap().scan,
            ScanStatus::NotScanned
        );
    }
    let out = plan(&spy, &cmp, &[("sub", L)], &COPY_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "keep_dir /l/sub > /r/sub @0",
            "create_dir /l/sub/deep > /r/sub/deep @1",
            "copy /l/sub/deep/y > /r/sub/deep/y @2",
            "copy /l/sub/x > /r/sub/x @1",
        ]
    );
    assert_eq!(*spy.seen.lock().unwrap(), ["/l/sub", "/l/sub/deep"]);
    // What a listing saw is stamped as a row is.
    assert!(out.plan.items.iter().all(|item| item.expect.is_some()));
    // What lies below the row has no row, and its paths are the bases'.
    assert_eq!(out.meta[0].node, Some(id(&cmp.read(), "sub")));
    assert!(out.meta[1..].iter().all(|meta| meta.node.is_none()));
    assert_eq!(out.meta[3].path, "sub/x");
}

/// Rules that leave `sub` unlisted, and with `filter` a name filter.
fn unlisted(filter: Option<&str>) -> FolderRules {
    let mut rules = FolderRules::default();
    rules.handling.auto_scan_subfolders = false;
    rules.handling.scan_orphan_folders = false;
    if let Some(text) = filter {
        let patch = FolderRulesPatch {
            name_filter: Some(text.to_owned()),
            ..FolderRulesPatch::default()
        };
        rules.apply(&patch).unwrap();
    }
    rules
}

fn read_only_at(secs: i64) -> MemMeta {
    MemMeta {
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: 0o444,
                uid: 0,
                gid: 0,
            },
        }),
        ..meta(secs)
    }
}

/// Rule 6 asks what a scanned row asks, from what it listed.
#[test]
fn a6_15_rule6_conflicts_are_those_of_a_row() {
    let vfs = memory();
    file(&vfs, "/l/sub/deep/x", b"old", 10);
    vfs.insert_file(&host("/r/sub/deep/x"), b"work".to_vec(), read_only_at(20))
        .unwrap();
    let rules = unlisted(None);
    let cmp = compared(&vfs, &rules);
    let at = |out: &OpPlan| {
        let index = out
            .meta
            .iter()
            .position(|meta| meta.path == "sub/deep/x")
            .unwrap();
        (out.plan.items[index].replaces, out.meta[index].conflicts)
    };
    let out = plan(&vfs, &cmp, &[("sub", L)], &COPY_RIGHT);
    assert_eq!(
        at(&out),
        (true, Conflicts::READ_ONLY | Conflicts::OVERWRITE_NEWER)
    );
    let out = plan(&vfs, &cmp, &[("sub", L)], &MOVE_RIGHT);
    assert_eq!(
        at(&out),
        (
            true,
            Conflicts::READ_ONLY | Conflicts::OVERWRITE_NEWER | Conflicts::REPLACE_DURING_MOVE
        )
    );
    // No to all keeps the target, as a row's.
    let out = plan(&vfs, &cmp, &[("sub", L)], &COPY_RIGHT);
    let done = Confirmation::new(&out, &ConfirmPolicy::ALL, ConfirmMode::NoToAll)
        .finish(out)
        .unwrap();
    let report = run(&vfs, done);
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/sub/deep/x").unwrap(), b"work");
    // Rule 9: `newer` reads the rules' tolerance from the same two times.
    let newer = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Newer,
    };
    let out = plan(&vfs, &cmp, &[("sub", L)], &newer);
    let index = out
        .meta
        .iter()
        .position(|m| m.path == "sub/deep/x")
        .unwrap();
    assert_eq!(out.plan.items[index].skip, Some(SkipReason::NotNewer));
}

/// A folder in the place of a folder of rule 6, and a file: rule 8, as a row.
#[test]
fn a6_15_rule6_a_folder_onto_a_file_is_rule_8() {
    let vfs = memory();
    vfs.insert_dir(&host("/l/sub/deep"), meta(10)).unwrap();
    file(&vfs, "/l/sub/deep/x", b"1", 10);
    file(&vfs, "/r/sub/deep", b"file", 20);
    let cmp = compared(&vfs, &unlisted(None));
    let out = plan(&vfs, &cmp, &[("sub", L)], &MOVE_RIGHT);
    let index = out.meta.iter().position(|m| m.path == "sub/deep").unwrap();
    assert!(out.plan.items[index].replaces);
    assert_eq!(
        out.meta[index].conflicts,
        Conflicts::OVERWRITE_NEWER | Conflicts::REPLACE_DURING_MOVE
    );
    let fail = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Fail,
    };
    let out = plan(&vfs, &cmp, &[("sub", L)], &fail);
    let line = lines(&out);
    assert!(
        line.iter().any(|line| line.contains("!already_exists")),
        "{line:?}"
    );
    assert!(
        !line.iter().any(|line| line.contains("/deep/x")),
        "{line:?}"
    );
}

/// D-176: what the filters hide is not touched, and a folder goes as one only when nothing below it stays.
#[test]
fn a6_15_rule6_filters_win() {
    let tree = || {
        let vfs = memory();
        file(&vfs, "/l/sub/seen.txt", b"1", 10);
        file(&vfs, "/l/sub/hidden.log", b"2", 10);
        file(&vfs, "/l/sub/clean/a.txt", b"3", 10);
        vfs
    };
    let rules = unlisted(Some("*.txt"));
    // A delete: no `remove_dir` of what stays, one of what does not.
    let vfs = tree();
    let cmp = compared(&vfs, &rules);
    {
        let read = cmp.read();
        assert_eq!(
            read.node(id(&read, "sub")).unwrap().scan,
            ScanStatus::NotScanned
        );
    }
    let out = plan(&vfs, &cmp, &[("sub", L)], &delete(L));
    assert_eq!(
        lines(&out),
        [
            "remove /l/sub/clean/a.txt @2",
            "remove_dir /l/sub/clean @1",
            "remove /l/sub/seen.txt @1",
        ]
    );
    let report = run(&vfs, out);
    assert!(report.ok());
    assert_eq!(read(&vfs, "/l/sub/hidden.log").unwrap(), b"2");
    // The bin: one `trash` of what is whole, the rest item by item.
    let vfs = tree();
    let cmp = compared(&vfs, &rules);
    let bin = FolderOp::Delete {
        sides: L,
        recycle: Recycle {
            use_bin: true,
            fallback: false,
        },
    };
    let out = plan(&vfs, &cmp, &[("sub", L)], &bin);
    assert_eq!(
        lines(&out),
        ["trash /l/sub/clean @1", "trash /l/sub/seen.txt @1"]
    );
    // A move: no `rename` of the folder, and the folder stays.
    let vfs = tree();
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("sub", L)], &MOVE_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/sub > /r/sub @0",
            "create_dir /l/sub/clean > /r/sub/clean @1",
            "move /l/sub/clean/a.txt > /r/sub/clean/a.txt @2",
            "remove_dir /l/sub/clean @1",
            "move /l/sub/seen.txt > /r/sub/seen.txt @1",
        ]
    );
    let report = run(&vfs, out);
    assert!(report.ok());
    assert_eq!(read(&vfs, "/l/sub/hidden.log").unwrap(), b"2");
    assert!(read(&vfs, "/r/sub/hidden.log").is_none());
    // A copy.
    let vfs = tree();
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("sub", L)], &COPY_RIGHT);
    assert!(lines(&out).iter().all(|line| !line.contains("hidden.log")));
}

/// `--include-hidden` takes what filters hide, and with nothing left out a folder is one item again.
#[test]
fn a6_15_rule6_include_hidden_takes_all() {
    let vfs = memory();
    file(&vfs, "/l/sub/seen.txt", b"1", 10);
    file(&vfs, "/l/sub/hidden.log", b"2", 10);
    let cmp = compared(&vfs, &unlisted(Some("*.txt")));
    let read_tree = cmp.read();
    let mut with = scope(&read_tree, &[("sub", L)]);
    with.include.hidden = true;
    let out = plan_op(
        &read_tree,
        &vfs,
        &with,
        &MOVE_RIGHT,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(lines(&out), ["rename /l/sub > /r/sub @0"]);
}

/// A filter on the side written keeps what stands there, as `skip filtered` keeps a row.
#[test]
fn a6_15_rule6_a_target_the_filters_hide_stays() {
    let vfs = memory();
    file(&vfs, "/l/sub/a.txt", b"small", 10);
    file(&vfs, "/r/sub/a.txt", &[0; 4096], 10);
    let mut rules = unlisted(None);
    let patch = FolderRulesPatch {
        size_filter: Some(Change::Set(SizeFilter {
            cmp: Cmp::Less,
            bytes: 1000,
        })),
        ..FolderRulesPatch::default()
    };
    rules.apply(&patch).unwrap();
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("sub", L)], &COPY_RIGHT);
    let line = lines(&out);
    assert!(line.iter().any(|l| l.ends_with("!filtered")), "{line:?}");
    assert!(!out.plan.items.iter().any(|item| item.replaces), "{line:?}");
}

/// What a listing saw is what runs: a file edited after the plan stays, with its folder, for a delete and a move.
#[test]
fn a6_15_rule6_items_are_stamped() {
    for op in [delete(L), MOVE_RIGHT.clone()] {
        let vfs = memory();
        file(&vfs, "/l/sub/x", b"1", 10);
        file(&vfs, "/l/sub/y", b"2", 10);
        vfs.set_device(&host("/r"), 9);
        let cmp = compared(&vfs, &unlisted(None));
        let out = plan(&vfs, &cmp, &[("sub", L)], &op);
        vfs.insert_file(&host("/l/sub/x"), b"edited".to_vec(), meta(30))
            .unwrap();
        let report = run(&vfs, out);
        assert!(!report.ok(), "{:?}", report.results);
        assert_eq!(report.summary.failed, 1);
        assert_eq!(read(&vfs, "/l/sub/x").unwrap(), b"edited");
        assert!(read(&vfs, "/l/sub/y").is_none());
    }
    // A replace: the target edited since is no target the plan saw.
    let vfs = memory();
    file(&vfs, "/l/sub/x", b"1", 10);
    file(&vfs, "/r/sub/x", b"2", 5);
    let cmp = compared(&vfs, &unlisted(None));
    let out = plan(&vfs, &cmp, &[("sub", L)], &COPY_RIGHT);
    vfs.insert_file(&host("/r/sub/x"), b"edited".to_vec(), meta(30))
        .unwrap();
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(read(&vfs, "/r/sub/x").unwrap(), b"edited");
}

#[test]
fn a6_15_rule6_no_directory_io_where_the_model_has_contents() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/l/a/b/y", b"2", 10);
    file(&vfs, "/r/a/x", b"3", 20);
    let cmp = compared(&vfs, &FolderRules::default());
    let spy = Lists {
        inner: &vfs,
        seen: std::sync::Mutex::new(Vec::new()),
    };
    for op in [&COPY_RIGHT, &MOVE_RIGHT, &delete(L)] {
        plan(&spy, &cmp, &[("a", L)], op);
    }
    assert_eq!(*spy.seen.lock().unwrap(), Vec::<String>::new());
}

#[test]
fn a6_15_rule6_cancelled_before_anything_is_written() {
    let vfs = memory();
    file(&vfs, "/l/a", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let before = vfs.dump();
    let cancel = CancelToken::new();
    cancel.cancel();
    let progress = hikasa_core::NullProgress;
    let ctl = Ctl {
        cancel: &cancel,
        progress: &progress,
    };
    let err = plan_op(
        &read,
        &vfs,
        &scope(&read, &[("a", L)]),
        &COPY_RIGHT,
        &OpSettings::default(),
        &ctl,
    )
    .unwrap_err();
    assert!(matches!(err, FolderError::Cancelled));
    assert_eq!(vfs.dump(), before);
}

#[test]
fn a6_15_rule6_a_name_only_entry_is_stated_for_its_stamp() {
    let vfs = memory();
    file(&vfs, "/l/x.bak", b"12345", 42);
    file(&vfs, "/l/keep.txt", b"k", 42);
    let mut rules = FolderRules::default();
    rules.filters.names.exclude_files = vec!["*.bak".to_owned()];
    let cmp = compared(&vfs, &rules);
    let read = cmp.read();
    let row = id(&read, "x.bak");
    let mut selected = scope(&read, &[("x.bak", L)]);
    selected.include.hidden = true;
    let view = read.node(row).unwrap();
    assert!(view.left.unwrap().stat_skipped);
    let out = plan_op(
        &read,
        &vfs,
        &selected,
        &COPY_RIGHT,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let stamp = out.plan.items[0].expect.unwrap().source.unwrap();
    assert_eq!(stamp.size, Some(5));
    assert_eq!(stamp.mtime, FileTime::from_unix(42, 0));
}

// ---- rule 7 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule7_a_whole_move_is_one_rename() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/l/a/b/y", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", L)], &MOVE_RIGHT);
    assert_eq!(lines(&out), ["rename /l/a > /r/a @0"]);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0);
    assert_eq!(read(&vfs, "/r/a/b/y").unwrap(), b"2");
    assert!(read(&vfs, "/l/a/x").is_none());
}

#[test]
fn a6_15_rule7_expansion_on_cross_device_at_execution() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    file(&vfs, "/l/a/b/y", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", L)], &MOVE_RIGHT);
    assert_eq!(lines(&out), ["rename /l/a > /r/a @0"]);
    // The disk moves under the plan.
    vfs.set_device(&host("/r"), 9);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let actions: Vec<&str> = report
        .plan
        .items
        .iter()
        .map(|i| i.action.as_str())
        .collect();
    assert!(actions.contains(&"move"), "{actions:?}");
    assert!(actions.contains(&"remove_dir"), "{actions:?}");
    assert_eq!(read(&vfs, "/r/a/x").unwrap(), b"1");
    assert!(read(&vfs, "/l/a/x").is_none());
}

#[test]
fn a6_15_rule7_devices_known_to_differ_go_row_by_row() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    vfs.set_device(&host("/r"), 9);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", L)], &MOVE_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/a > /r/a @0",
            "move /l/a/x > /r/a/x @1",
            "remove_dir /l/a @0",
        ]
    );
}

#[test]
fn a6_15_rule7_without_whole_hidden_items_stay_and_so_does_their_folder() {
    let vfs = memory();
    file(&vfs, "/l/a/x.txt", b"1", 10);
    file(&vfs, "/l/a/y.bak", b"2", 10);
    let mut rules = FolderRules::default();
    rules.filters.names.exclude_files = vec!["*.bak".to_owned()];
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("a", L)], &MOVE_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "create_dir /l/a > /r/a @0",
            "move /l/a/x.txt > /r/a/x.txt @1"
        ]
    );
    let out = plan(&vfs, &cmp, &[("a", L)], &delete(L));
    assert_eq!(lines(&out), ["remove /l/a/x.txt @0"]);
}

// ---- rule 8 --------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule8_a_folder_at_the_name_of_a_file_fails_whatever_the_policy() {
    let vfs = memory();
    file(&vfs, "/l/same", b"1", 10);
    file(&vfs, "/r/same/inner", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    for overwrite in OverwritePolicy::ALL {
        let op = FolderOp::Copy {
            to: Side::Right,
            overwrite,
        };
        let read = cmp.read();
        let row = read
            .children(read.root())
            .find(|&row| read.node(row).unwrap().left.is_some())
            .unwrap();
        let mut selected = scope(&read, &[]);
        selected.selection.insert(row, L);
        let out = plan_op(
            &read,
            &vfs,
            &selected,
            &op,
            &OpSettings::default(),
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(lines(&out), ["copy /l/same > /r/same @0 !is_a_directory"]);
    }
}

#[test]
fn a6_15_rule8_a_file_at_the_name_of_a_folder() {
    let vfs = memory();
    file(&vfs, "/l/same/inner", b"1", 10);
    file(&vfs, "/r/same", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let folder_row = |read: &TreeRead<'_>| {
        read.children(read.root())
            .find(|&row| read.node(row).unwrap().is_folder())
            .unwrap()
    };
    let plan_for = |overwrite| {
        let op = FolderOp::Copy {
            to: Side::Right,
            overwrite,
        };
        let read = cmp.read();
        let mut selected = scope(&read, &[]);
        selected.selection.insert(folder_row(&read), L);
        let settings = OpSettings {
            backup: Some(BackupPolicy {
                style: BackupStyle::NameBak,
                folder: None,
            }),
            ..OpSettings::default()
        };
        plan_op(&read, &vfs, &selected, &op, &settings, &Ctl::detached()).unwrap()
    };
    for overwrite in [
        OverwritePolicy::Fail,
        OverwritePolicy::Skip,
        OverwritePolicy::Newer,
    ] {
        let out = plan_for(overwrite);
        assert_eq!(
            lines(&out)[0],
            "create_dir /l/same > /r/same @0 !already_exists"
        );
    }
    let out = plan_for(OverwritePolicy::Always);
    assert_eq!(lines(&out)[0], "create_dir /l/same > /r/same @0 +replaces");
    assert_eq!(
        short(out.plan.items[0].backup.as_ref().unwrap()),
        "/r/same.bak"
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/same/inner").unwrap(), b"1");
    assert_eq!(read(&vfs, "/r/same.bak").unwrap(), b"2");
}

#[test]
fn a6_15_rule8_what_fails_cuts_what_lies_below_it() {
    let vfs = memory();
    file(&vfs, "/l/same/inner", b"1", 10);
    file(&vfs, "/r/same", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let folder = read
        .children(read.root())
        .find(|&row| read.node(row).unwrap().is_folder())
        .unwrap();
    let mut selected = scope(&read, &[]);
    selected.selection.insert(folder, L);
    let op = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Fail,
    };
    let out = plan_op(
        &read,
        &vfs,
        &selected,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read);
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(report.results[1].status, ItemStatus::NotRun);
    assert_eq!(crate::read(&vfs, "/r/same").unwrap(), b"2");
}

// ---- rule 9 --------------------------------------------------------------------------------------------------

fn overwrite_tree() -> MemoryVfs {
    let vfs = memory();
    file(&vfs, "/l/newer", b"left", 20);
    file(&vfs, "/r/newer", b"right", 10);
    file(&vfs, "/l/older", b"left", 10);
    file(&vfs, "/r/older", b"right", 20);
    vfs
}

#[test]
fn a6_15_rule9_each_policy() {
    let vfs = overwrite_tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let rows = [("newer", L), ("older", L)];
    let of = |overwrite| {
        let op = FolderOp::Copy {
            to: Side::Right,
            overwrite,
        };
        lines(&plan(&vfs, &cmp, &rows, &op))
    };
    assert_eq!(
        of(OverwritePolicy::Fail),
        [
            "copy /l/newer > /r/newer @0 !already_exists",
            "copy /l/older > /r/older @0 !already_exists"
        ]
    );
    assert_eq!(
        of(OverwritePolicy::Skip),
        [
            "skip /l/newer > /r/newer @0 !exists",
            "skip /l/older > /r/older @0 !exists"
        ]
    );
    assert_eq!(
        of(OverwritePolicy::Always),
        [
            "copy /l/newer > /r/newer @0 +replaces",
            "copy /l/older > /r/older @0 +replaces"
        ]
    );
    assert_eq!(
        of(OverwritePolicy::Newer),
        [
            "copy /l/newer > /r/newer @0 +replaces",
            "skip /l/older > /r/older @0 !not_newer"
        ]
    );
}

#[test]
fn a6_15_rule9_newer_uses_the_tolerance_and_unknown_is_not_newer() {
    let vfs = memory();
    file(&vfs, "/l/near", b"left", 12);
    file(&vfs, "/r/near", b"right", 10);
    vfs.insert_file(&host("/l/none"), b"x".to_vec(), MemMeta::default())
        .unwrap();
    file(&vfs, "/r/none", b"y", 10);
    let mut rules = FolderRules::default();
    rules.criteria.timestamps.tolerance_secs = 5;
    let cmp = compared(&vfs, &rules);
    let op = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Newer,
    };
    assert_eq!(
        lines(&plan(&vfs, &cmp, &[("near", L), ("none", L)], &op)),
        [
            "skip /l/near > /r/near @0 !not_newer",
            "skip /l/none > /r/none @0 !not_newer"
        ]
    );
}

#[test]
fn a6_15_rule9_the_same_file_under_two_names_is_no_replacement() {
    // Bases that are one folder: a move onto itself would lose the file.
    let vfs = memory();
    file(&vfs, "/l/x", b"only copy", 10);
    let engines = engines(1 << 30);
    let cmp = with_env(&vfs, &engines, &CancelToken::new(), |env| {
        let model = FolderModel::new(mspec("/l"), mspec("/l"), FolderRules::default()).unwrap();
        compare_into(&model, env).unwrap()
    });
    let out = plan(&vfs, &cmp, &[("x", L)], &MOVE_RIGHT);
    assert_eq!(lines(&out), ["move /l/x > /l/x @0 !invalid_input"]);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(read(&vfs, "/l/x").unwrap(), b"only copy");
}

// ---- rule 10 -------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule10_stamps() {
    let vfs = memory();
    file(&vfs, "/l/f", b"12345", 20);
    file(&vfs, "/r/f", b"123", 10);
    vfs.insert_dir(&host("/l/d"), meta(5)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("f", L), ("d", L)], &COPY_RIGHT);
    let dir = ItemStamp {
        kind: EntryKind::Dir,
        size: None,
        mtime: None,
    };
    assert_eq!(
        out.plan.items[0].expect,
        Some(Expect {
            source: Some(dir),
            target: None
        })
    );
    assert_eq!(
        out.plan.items[1].expect,
        Some(Expect {
            source: Some(ItemStamp {
                kind: EntryKind::File,
                size: Some(5),
                mtime: FileTime::from_unix(20, 0),
            }),
            target: Some(ItemStamp {
                kind: EntryKind::File,
                size: Some(3),
                mtime: FileTime::from_unix(10, 0),
            }),
        })
    );
    let out = plan(&vfs, &cmp, &[("d", L)], &delete(L));
    assert_eq!(
        out.plan.items[0].expect,
        Some(Expect {
            source: None,
            target: Some(dir)
        })
    );
}

#[test]
fn a6_15_rule10_a_file_changed_since_the_comparison_is_not_overwritten() {
    let vfs = overwrite_tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("newer", L)], &COPY_RIGHT);
    file(&vfs, "/r/newer", b"edited after the compare", 99);
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/r/newer").unwrap(), b"edited after the compare");
}

#[test]
fn a6_15_rule10_a_followed_target_stamps_the_link_by_kind() {
    let vfs = memory();
    file(&vfs, "/elsewhere/data", b"old", 10);
    file(&vfs, "/l/f", b"new", 20);
    vfs.insert_symlink(&host("/r/f"), &host("/elsewhere/data"), meta(10))
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.follow_symlinks = true;
    let cmp = compared(&vfs, &rules);
    let out = plan(&vfs, &cmp, &[("f", L)], &COPY_RIGHT);
    let stamp = out.plan.items[0].expect.unwrap().target.unwrap();
    assert_eq!(stamp.kind, EntryKind::Symlink);
    assert_eq!((stamp.size, stamp.mtime), (None, None));
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    // The replace acts on the link: what it led to stays.
    assert_eq!(read(&vfs, "/elsewhere/data").unwrap(), b"old");
}

// ---- rule 11 -------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule11_a_side_without_the_bit_skips_with_one_warning() {
    let vfs = memory();
    file(&vfs, "/l/a", b"1", 10);
    file(&vfs, "/l/b", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let before = vfs.capabilities(&mspec("/l")).unwrap();
    vfs.set_capabilities(before.difference(Capabilities::WRITE));
    let out = plan(&vfs, &cmp, &[("a", L), ("b", L)], &COPY_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "skip /l/a > /r/a @0 !capability",
            "skip /l/b > /r/b @0 !capability"
        ]
    );
    let warnings: Vec<&Warning> = out
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::CapabilityMissing)
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("right"), "{warnings:?}");
}

// ---- rule 12 -------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule12_below_a_followed_folder_nothing_is_written_or_deleted() {
    let vfs = memory();
    file(&vfs, "/elsewhere/d/x", b"out", 10);
    file(&vfs, "/l/d/x", b"in", 20);
    vfs.insert_symlink(&host("/r/d"), &host("/elsewhere/d"), meta(10))
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.follow_symlinks = true;
    let cmp = compared(&vfs, &rules);
    let before = read(&vfs, "/elsewhere/d/x").unwrap();
    assert_eq!(
        lines(&plan(&vfs, &cmp, &[("d/x", L)], &COPY_RIGHT)),
        ["skip /l/d/x > /r/d/x @0 !followed_link"]
    );
    assert_eq!(
        lines(&plan(&vfs, &cmp, &[("d/x", R)], &delete(R))),
        ["skip /r/d/x @0 !followed_link"]
    );
    assert_eq!(before, b"out");
    // The link itself: a delete removes the link.
    let out = plan(&vfs, &cmp, &[("d", R)], &delete(R));
    assert_eq!(lines(&out), ["remove /r/d @0"]);
    assert_eq!(out.plan.items[0].kind, EntryKind::Symlink);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/elsewhere/d/x").unwrap(), b"out");
}

#[test]
fn a6_15_rule12_rows_that_failed_are_never_touched() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"in", 20);
    file(&vfs, "/r/d/x", b"other", 20);
    file(&vfs, "/l/g", b"good", 20);
    vfs.inject(Fault {
        path: host("/r/d"),
        op: VfsOp::List,
        kind: ErrorKind::PermissionDenied,
    });
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let d = id(&read, "d");
    assert_eq!(read.node(d).unwrap().scan, ScanStatus::Failed);
    let rows: Vec<NodeId> = read.children(d).collect();
    let mut selected = scope(&read, &[("g", L)]);
    selected.selection.insert(d, L);
    for row in rows {
        selected.selection.insert(row, L);
    }
    let out = plan_op(
        &read,
        &vfs,
        &selected,
        &delete(L),
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let text = lines(&out);
    assert!(text.contains(&"remove /l/g @0".to_owned()), "{text:?}");
    assert!(
        text.iter()
            .filter(|l| !l.contains("/l/g"))
            .all(|l| l.contains("!compare_error")),
        "{text:?}"
    );
}

// ---- rule 13 -------------------------------------------------------------------------------------------------

#[test]
fn a6_15_rule13_exchange_a_pair_moves_an_orphan_in_preorder() {
    let vfs = memory();
    file(&vfs, "/l/a/p", b"lp", 10);
    file(&vfs, "/r/a/p", b"rprp", 20);
    file(&vfs, "/l/a/lonly", b"only", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("a", SideSet::all())], &FolderOp::Exchange);
    assert_eq!(
        lines(&out),
        [
            "move /l/a/lonly > /r/a/lonly @0",
            "exchange /l/a/p > /r/a/p @0 +replaces",
        ]
    );
    assert_eq!(out.plan.bytes(), 2 + 4 + 4);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/l/a/p").unwrap(), b"rprp");
    assert_eq!(read(&vfs, "/r/a/p").unwrap(), b"lp");
    assert_eq!(read(&vfs, "/r/a/lonly").unwrap(), b"only");
}

// ---- rules 14 to 16 are A6.19, in `ops_touch_mkdir_copyto.rs`; rule 17 -------------------------------------------

#[test]
fn a6_15_rule17_a_plan_of_members_needs_nothing_more() {
    let vfs = memory();
    file(&vfs, "/l/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("x", L)], &COPY_RIGHT);
    assert!(out.exec.archive_backups.is_empty());
    assert_eq!(out.exec.fence.as_ref().unwrap().bases.len(), 2);
    assert!(out.exec.fence.as_ref().unwrap().writes);
}

// ---- the plan as data ----------------------------------------------------------------------------------------

#[test]
fn a6_15_the_plan_never_writes_is_clone_and_reads_back_the_same() {
    let vfs = overwrite_tree();
    file(&vfs, "/l/dir/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let before = vfs.dump();
    let settings = OpSettings {
        backup: Some(BackupPolicy {
            style: BackupStyle::NameBak,
            folder: None,
        }),
        ..OpSettings::default()
    };
    let out = plan_with(
        &vfs,
        &cmp,
        &[("newer", L), ("dir", L)],
        &COPY_RIGHT,
        &settings,
    );
    assert_eq!(vfs.dump(), before);
    let text = serde_json::to_string(&out).unwrap();
    let back: OpPlan = serde_json::from_str(&text).unwrap();
    assert_eq!(back, out.clone());
    let report = run(&vfs, back);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/newer").unwrap(), b"left");
    assert_eq!(read(&vfs, "/r/newer.bak").unwrap(), b"right");
    assert_eq!(read(&vfs, "/r/dir/x").unwrap(), b"1");
}

#[test]
fn a6_15_backups_number_around_the_plan() {
    let vfs = memory();
    file(&vfs, "/l/x", b"new", 20);
    file(&vfs, "/r/x", b"old", 10);
    // `x.bak` is the target of the next row.
    file(&vfs, "/l/x.bak", b"new2", 20);
    file(&vfs, "/r/x.bak", b"old2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let settings = OpSettings {
        backup: Some(BackupPolicy {
            style: BackupStyle::NameBak,
            folder: None,
        }),
        ..OpSettings::default()
    };
    let out = plan_with(
        &vfs,
        &cmp,
        &[("x", L), ("x.bak", L)],
        &COPY_RIGHT,
        &settings,
    );
    let backups: Vec<String> = out
        .plan
        .items
        .iter()
        .map(|item| short(item.backup.as_ref().unwrap()))
        .collect();
    assert_eq!(backups, ["/r/x-2.bak", "/r/x.bak.bak"]);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/x-2.bak").unwrap(), b"old");
    assert_eq!(read(&vfs, "/r/x.bak.bak").unwrap(), b"old2");
    assert_eq!(read(&vfs, "/r/x.bak").unwrap(), b"new2");
}

#[test]
fn a6_15_a_folder_at_a_backups_name_fails_that_item_at_planning() {
    let vfs = memory();
    file(&vfs, "/l/x", b"new", 20);
    file(&vfs, "/r/x", b"old", 10);
    vfs.insert_dir(&host("/r/x.bak"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let settings = OpSettings {
        backup: Some(BackupPolicy {
            style: BackupStyle::NameBak,
            folder: None,
        }),
        ..OpSettings::default()
    };
    let out = plan_with(&vfs, &cmp, &[("x", L)], &COPY_RIGHT, &settings);
    assert_eq!(lines(&out), ["copy /l/x > /r/x @0 !is_a_directory"]);
}

#[test]
fn a6_15_duplicates_of_a_case_insensitive_target() {
    // Two orphans that one folder of the other side folds to one name.
    let vfs = memory();
    file(&vfs, "/l/A", b"1", 10);
    file(&vfs, "/l/a", b"2", 10);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Sensitive;
    let cmp = compared(&vfs, &rules);
    let before = vfs.capabilities(&mspec("/l")).unwrap();
    vfs.set_capabilities(before.difference(Capabilities::CASE_SENSITIVE));
    let out = plan(&vfs, &cmp, &[("A", L), ("a", L)], &COPY_RIGHT);
    assert_eq!(
        lines(&out),
        [
            "copy /l/A > /r/A @0",
            "skip /l/a > /r/a @0 !duplicate_target"
        ]
    );
}

proptest! {
    #![proptest_config(config_with("tests/proptest-regressions/ops_plan.txt", 64))]

    /// Whatever the attributes, a planned stamp agrees with a stat of what the model saw.
    #[test]
    fn a6_15_stamps_hold_on_an_unchanged_tree(secs in 1i64..100, size in 0usize..9) {
        let vfs = memory();
        let mut with = meta(secs);
        with.attrs = Some(Attrs::Unix { meta: UnixMeta { mode: 0o644, uid: 0, gid: 0 } });
        vfs.insert_file(&host("/l/x"), vec![b'x'; size], with).unwrap();
        file(&vfs, "/r/x", b"old", 1);
        let cmp = compared(&vfs, &FolderRules::default());
        let out = plan(&vfs, &cmp, &[("x", L)], &COPY_RIGHT);
        let report = run(&vfs, out);
        prop_assert_eq!(report.summary.failed, 0);
    }
}

/// A folder on the way that a filter keeps: what goes in it is skipped for the same reason, at planning (P-OPS).
#[test]
fn a6_15_what_lies_in_a_skipped_folder_is_skipped_too() {
    let vfs = memory();
    vfs.insert_dir(&host("/l/a"), meta(10)).unwrap();
    file(&vfs, "/r/a", b"hidden", 10);
    let cmp = compared(&vfs, &unlisted(Some("*.txt")));
    let parent = id(&cmp.read(), "a");
    let op = FolderOp::Mkdir {
        sides: SideSet::all(),
        parent,
        name: "new".to_owned(),
        folder: true,
    };
    let out = plan(&vfs, &cmp, &[], &op);
    let line = lines(&out);
    assert!(
        line.iter()
            .filter(|l| l.contains("/r/a/new"))
            .all(|l| l.ends_with("!filtered")),
        "{line:?}"
    );
    let report = run(&vfs, out);
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(read(&vfs, "/r/a").unwrap(), b"hidden");
}
