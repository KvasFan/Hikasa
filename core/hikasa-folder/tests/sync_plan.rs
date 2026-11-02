//! M2 W6c acceptance A6.27 (D-182, D-166): the rules of a sync plan on `MemoryVfs`. The row algorithm of §1.6.14 one step at
//! a time, conflicts, Ignored rows, overrides, folders, the order of the batch, the effective rules, the refusals (an empty
//! source, a comparison that did not finish), rows the comparison could not tell, read-only sides, the folder summaries,
//! archives, and a tree that changed since the comparison. Every test that runs a plan checks the disk afterwards.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use common::{CancelAfter, compared, default_now, engines, host, mspec, run_on};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, NullProgress, RelPath, Side, Spec,
    WarningCode,
};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, OpSettings, QuestionKind, ReadOnlySides,
};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::sync::{
    LeaveReason, OrphanReading, Situation, SyncAction, SyncCounts, SyncMatrix, SyncOptions,
    SyncPlan, SyncPreset, SyncRequest, SyncRow, plan_sync, sync_folder_rules,
};
use hikasa_folder::{
    ArchiveHandling, CaseMode, Cmp, DisplayOptions, FolderComparison, FolderDisplayFilter,
    FolderEnv, FolderError, FolderMode, FolderModel, FolderRules, FolderRulesPatch, NodeId,
    NullObserver, SizeFilter, TreeRead, compare_into,
};
use hikasa_vfs::ops::{self, Action, BackupPolicy, BackupStyle, ItemStatus, OpReport, SkipReason};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, EntryKind, Fault,
    MemMeta, MemoryVfs, Vfs, VfsOp,
};

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn memory(case: CaseSensitivity) -> MemoryVfs {
    let vfs = MemoryVfs::new(case);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    vfs
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content.to_vec(), meta(secs))
        .unwrap();
}

fn dir(vfs: &MemoryVfs, path: &str) {
    vfs.insert_dir(&host(path), meta(1)).unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn exists(vfs: &MemoryVfs, path: &str) -> bool {
    vfs.dump().iter().any(|entry| entry.path == host(path))
}

fn preset(preset: SyncPreset) -> SyncOptions {
    SyncOptions {
        preset,
        ..SyncOptions::default()
    }
}

fn request(options: &SyncOptions) -> SyncRequest<'_> {
    SyncRequest {
        options,
        just_selection: None,
        visible: None,
        include_hidden: false,
        iu: false,
        read_only: ReadOnlySides::default(),
        allow_empty_source: false,
        settings: OpSettings::default(),
    }
}

/// The comparison a sync makes: its effective rules over `rules`.
fn sync_compare(vfs: &dyn Vfs, rules: &FolderRules, options: &SyncOptions) -> FolderComparison {
    compared(vfs, &sync_folder_rules(rules, options).unwrap())
}

fn plan(vfs: &dyn Vfs, cmp: &FolderComparison, request: &SyncRequest<'_>) -> SyncPlan {
    plan_sync(&cmp.read(), vfs, request, &Ctl::detached()).unwrap()
}

/// Runs a plan with every question answered yes.
fn execute(vfs: &dyn Vfs, plan: &SyncPlan) -> OpReport {
    let op = Confirmation::new(&plan.op, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(plan.op.clone())
        .unwrap();
    ops::execute_with(vfs, vfs, op.plan, &op.exec, &Ctl::detached())
}

fn assert_clean(report: &OpReport) {
    for (item, result) in report.plan.items.iter().zip(&report.results) {
        assert!(
            matches!(result.status, ItemStatus::Done | ItemStatus::Skipped),
            "{:?} {} {result:?}",
            item.action,
            item.target.canonical()
        );
    }
}

fn id(read: &TreeRead<'_>, path: &str) -> NodeId {
    read.node_at(&RelPath::parse(path).unwrap())
        .unwrap_or_else(|| panic!("no row {path}"))
}

/// The row of `path`; a file and a folder of one name take the kind: `x/` is the folder.
fn row(cmp: &FolderComparison, plan: &SyncPlan, path: &str) -> SyncRow {
    let read = cmp.read();
    let (path, folder) = match path.strip_suffix('/') {
        Some(path) => (path, true),
        None => (path, false),
    };
    let first = id(&read, path);
    let parent = read
        .node(first)
        .and_then(|v| v.parent)
        .unwrap_or(NodeId::ROOT);
    let node = read
        .children(parent)
        .find(|&child| {
            read.path(child).as_str() == path
                && read.node(child).is_some_and(|v| v.is_folder() == folder)
        })
        .unwrap_or(first);
    *plan
        .rows
        .iter()
        .find(|row| row.node == node)
        .unwrap_or_else(|| panic!("no plan row {path}"))
}

fn short(spec: &Spec) -> String {
    let text = spec.canonical().replace('\\', "/");
    text.strip_prefix("C:").map(str::to_owned).unwrap_or(text)
}

fn items(plan: &SyncPlan) -> Vec<String> {
    plan.op
        .plan
        .items
        .iter()
        .map(|item| {
            let mut line = format!("{} {}", item.action.as_str(), short(&item.target));
            if let Some(reason) = item.skip {
                line.push_str(&format!(" !{}", reason.as_str()));
            }
            line
        })
        .collect()
}

// ---- step 1: scope ---------------------------------------------------------------------------------------------

#[test]
fn a6_27_rule1_just_selection_and_visible_keep_the_rest() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    file(&vfs, "/l/b.txt", b"b", 10);
    file(&vfs, "/l/c.txt", b"new c", 20);
    file(&vfs, "/r/c.txt", b"old c", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let mut selection = Selection::default();
    selection.insert(id(&cmp.read(), "a.txt"), SideSet::LEFT);
    let mut req = request(&options);
    req.just_selection = Some(&selection);
    let out = plan(&vfs, &cmp, &req);
    assert_eq!(row(&cmp, &out, "a.txt").action, SyncAction::CopyRight);
    for path in ["b.txt", "c.txt"] {
        let r = row(&cmp, &out, path);
        assert_eq!(
            (r.action, r.reason),
            (SyncAction::Leave, Some(LeaveReason::OutOfScope))
        );
    }
    assert_clean(&execute(&vfs, &out));
    assert!(exists(&vfs, "/r/a.txt") && !exists(&vfs, "/r/b.txt"));
    assert_eq!(read(&vfs, "/r/c.txt").unwrap(), b"old c");

    // `sync visible`: what the display shows.
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    file(&vfs, "/l/c.txt", b"new c", 20);
    file(&vfs, "/r/c.txt", b"old c", 10);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let mut req = request(&options);
    req.visible = Some(DisplayOptions {
        filter: FolderDisplayFilter::LeftNewer,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    });
    let out = plan(&vfs, &cmp, &req);
    assert_eq!(row(&cmp, &out, "c.txt").action, SyncAction::CopyRight);
    assert_eq!(
        row(&cmp, &out, "a.txt").reason,
        Some(LeaveReason::OutOfScope)
    );
}

// ---- step 2: what the comparison could not tell ---------------------------------------------------------------

#[test]
fn a6_27_rule2_a_folder_that_failed_on_the_source_deletes_nothing_below_it() {
    let vfs = memory(CaseSensitivity::Sensitive);
    dir(&vfs, "/l/d");
    file(&vfs, "/l/d/x.txt", b"left", 10);
    dir(&vfs, "/r/d");
    file(&vfs, "/r/d/x.txt", b"right", 10);
    file(&vfs, "/r/d/only.txt", b"keep me", 10);
    file(&vfs, "/r/gone.txt", b"orphan", 10);
    vfs.inject(Fault {
        path: host("/l/d"),
        op: VfsOp::List,
        kind: ErrorKind::PermissionDenied,
    });
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let d = row(&cmp, &out, "d/");
    assert_eq!(d.reason, Some(LeaveReason::CompareError));
    assert!(out.counts.unresolved >= 1);
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == WarningCode::SyncIncomplete),
        "{:?}",
        out.warnings
    );
    // Nothing of `d` is in the plan; the orphan elsewhere goes.
    assert!(
        items(&out).iter().all(|line| !line.contains("/r/d")),
        "{:?}",
        items(&out)
    );
    vfs.clear_faults();
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/d/only.txt").unwrap(), b"keep me");
    assert_eq!(read(&vfs, "/r/d/x.txt").unwrap(), b"right");
    assert!(!exists(&vfs, "/r/gone.txt"));
}

#[test]
fn a6_27_rule2_an_error_row_and_an_unscanned_folder_are_left() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/e.txt", b"left", 10);
    file(&vfs, "/r/e.txt", b"right side", 10);
    dir(&vfs, "/r/u");
    file(&vfs, "/r/u/in.txt", b"unlisted", 10);
    // A content test that fails: the row is an error, not a difference.
    let mut rules = FolderRules::default();
    rules.criteria.content.method = hikasa_core::ContentMethod::Binary;
    vfs.inject(Fault {
        path: host("/r/e.txt"),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    // A model compared without the sync's rules: the orphan folder was not scanned.
    rules.handling.scan_orphan_folders = false;
    let cmp = compared(&vfs, &rules);
    vfs.clear_faults();
    let options = preset(SyncPreset::MirrorRight);
    let out = plan(&vfs, &cmp, &request(&options));
    for path in ["e.txt", "u/"] {
        let r = row(&cmp, &out, path);
        assert_eq!(
            (r.action, r.reason),
            (SyncAction::Leave, Some(LeaveReason::CompareError)),
            "{path}"
        );
    }
    assert_eq!(out.counts.unresolved, 2);
    assert!(out.op.plan.items.is_empty(), "{:?}", items(&out));
    execute(&vfs, &out);
    assert_eq!(read(&vfs, "/r/e.txt").unwrap(), b"right side");
    assert_eq!(read(&vfs, "/r/u/in.txt").unwrap(), b"unlisted");
}

// ---- step 3: filters ----------------------------------------------------------------------------------------------

#[test]
fn a6_27_rule3_a_side_a_filter_hides_is_left_unless_asked() {
    let setup = || {
        let vfs = memory(CaseSensitivity::Sensitive);
        // The left side is hidden by the size filter, the right one is not.
        file(&vfs, "/l/f.txt", b"tiny", 10);
        file(&vfs, "/r/f.txt", b"large enough", 20);
        // A folder excluded by name, on the right only: a mirror must not take it.
        dir(&vfs, "/r/build");
        file(&vfs, "/r/build/out.o", b"object", 10);
        vfs
    };
    let mut rules = FolderRules::default();
    rules.filters.other.size = Some(SizeFilter {
        cmp: Cmp::Greater,
        bytes: 5,
    });
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some(r"*;-build\".into()),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let options = preset(SyncPreset::MirrorRight);
    let vfs = setup();
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(row(&cmp, &out, "f.txt").reason, Some(LeaveReason::Filtered));
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/f.txt").unwrap(), b"large enough");
    assert_eq!(read(&vfs, "/r/build/out.o").unwrap(), b"object");
    // With `--include-hidden` the hidden side counts.
    let vfs = setup();
    let cmp = sync_compare(&vfs, &rules, &options);
    let mut req = request(&options);
    req.include_hidden = true;
    let out = plan(&vfs, &cmp, &req);
    assert_eq!(
        row(&cmp, &out, "f.txt").situation,
        Some(Situation::NewerRight)
    );
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/f.txt").unwrap(), b"tiny");
}

#[test]
fn a6_27_rule3_a_folder_hidden_on_the_source_hides_its_rows_on_the_target() {
    let vfs = memory(CaseSensitivity::Sensitive);
    // The left folder is protected (system and hidden): the default filter hides it on the left only, and its pair is
    // listed. What the right has in it is no orphan a mirror may delete.
    vfs.insert_dir(
        &host("/l/p"),
        MemMeta {
            attrs: Some(hikasa_vfs::Attrs::Dos {
                attrs: hikasa_vfs::DosAttrs::SYSTEM | hikasa_vfs::DosAttrs::HIDDEN,
            }),
            ..meta(1)
        },
    )
    .unwrap();
    file(&vfs, "/l/p/same.txt", b"s", 10);
    dir(&vfs, "/r/p");
    file(&vfs, "/r/p/same.txt", b"s", 10);
    file(&vfs, "/r/p/orphan.txt", b"keep", 10);
    file(&vfs, "/l/a.txt", b"a", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(
        row(&cmp, &out, "p/orphan.txt").reason,
        Some(LeaveReason::Filtered)
    );
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/p/orphan.txt").unwrap(), b"keep");
}

// ---- step 4: situations, Ignored ------------------------------------------------------------------------------------

#[test]
fn a6_27_rule4_an_ignored_row_is_same() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/r/keep.txt", b"ignored orphan", 10);
    dir(&vfs, "/r/vendor");
    file(&vfs, "/r/vendor/lib.c", b"ignored", 10);
    file(&vfs, "/l/x.txt", b"newer", 20);
    file(&vfs, "/r/x.txt", b"older", 10);
    let rules = FolderRules {
        ignored: vec!["keep.txt".into(), "vendor".into(), "x.txt".into()],
        ..FolderRules::default()
    };
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    for path in ["keep.txt", "vendor/lib.c", "x.txt"] {
        let r = row(&cmp, &out, path);
        assert_eq!(
            (r.situation, r.action, r.reason),
            (
                Some(Situation::Same),
                SyncAction::Leave,
                Some(LeaveReason::Same)
            ),
            "{path}"
        );
    }
    assert!(out.op.plan.items.is_empty(), "{:?}", items(&out));
}

// ---- step 5: overrides ---------------------------------------------------------------------------------------------

#[test]
fn a6_27_rule5_overrides_a_path_a_folder_and_one_that_cannot_apply() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    dir(&vfs, "/l/d");
    file(&vfs, "/l/d/one.txt", b"1", 10);
    file(&vfs, "/l/d/two.txt", b"2", 10);
    file(&vfs, "/l/d/three.txt", b"3", 10);
    file(&vfs, "/r/z.txt", b"z", 10);
    let mut options = preset(SyncPreset::MirrorRight);
    options.overrides = BTreeMap::from([
        ("a.txt".to_owned(), SyncAction::Leave),
        // A folder covers its subtree; the longest path decides.
        ("d".to_owned(), SyncAction::Leave),
        ("d/two.txt".to_owned(), SyncAction::CopyRight),
        // Left-only: no right side to copy from. Dropped, with a warning; the matrix decides.
        ("d/three.txt".to_owned(), SyncAction::CopyLeft),
        ("nowhere/x".to_owned(), SyncAction::DeleteLeft),
    ]);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let a = row(&cmp, &out, "a.txt");
    assert_eq!(
        (a.action, a.overridden, a.reason),
        (SyncAction::Leave, true, Some(LeaveReason::Matrix))
    );
    assert_eq!(row(&cmp, &out, "d/one.txt").action, SyncAction::Leave);
    assert!(row(&cmp, &out, "d/one.txt").overridden);
    let two = row(&cmp, &out, "d/two.txt");
    assert_eq!((two.action, two.overridden), (SyncAction::CopyRight, true));
    let three = row(&cmp, &out, "d/three.txt");
    assert_eq!(
        (three.action, three.overridden),
        (SyncAction::CopyRight, false)
    );
    assert_eq!(row(&cmp, &out, "z.txt").action, SyncAction::DeleteRight);
    let ignored: Vec<&str> = out
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::SwitchIgnored)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(ignored.len(), 2, "{ignored:?}");
    assert!(ignored.iter().any(|m| m.contains("d/three.txt")));
    assert!(ignored.iter().any(|m| m.contains("nowhere/x")));
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/a.txt") && !exists(&vfs, "/r/d/one.txt"));
    assert!(exists(&vfs, "/r/d/two.txt") && exists(&vfs, "/r/d/three.txt"));
    // An override of the base, `.` or `..` is refused before anything runs.
    for bad in ["", ".", "/", "a/../b", "./a"] {
        let mut options = preset(SyncPreset::MirrorRight);
        options.overrides.insert(bad.to_owned(), SyncAction::Leave);
        assert!(
            sync_folder_rules(&FolderRules::default(), &options).is_err(),
            "{bad:?}"
        );
        let err = plan_sync(&cmp.read(), &vfs, &request(&options), &Ctl::detached()).unwrap_err();
        assert!(matches!(err, FolderError::InvalidRules(_)), "{bad:?}");
    }
}

// ---- step 6: what the side written allows -------------------------------------------------------------------------

#[test]
fn a6_27_rule6_a_read_only_side_is_left_row_by_row() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/new.txt", b"new on the left", 10);
    file(&vfs, "/r/old.txt", b"right only", 10);
    file(&vfs, "/r/up.txt", b"newer on the right", 20);
    file(&vfs, "/l/up.txt", b"older", 10);
    let options = preset(SyncPreset::UpdateBoth);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let mut req = request(&options);
    req.read_only = ReadOnlySides {
        left: false,
        right: true,
    };
    let out = plan(&vfs, &cmp, &req);
    assert_eq!(
        row(&cmp, &out, "new.txt").reason,
        Some(LeaveReason::ReadOnlySide)
    );
    // The other side goes on.
    assert_eq!(row(&cmp, &out, "old.txt").action, SyncAction::CopyLeft);
    assert_eq!(row(&cmp, &out, "up.txt").action, SyncAction::CopyLeft);
    let warned: Vec<_> = out
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::ReadOnlySide)
        .collect();
    assert_eq!(warned.len(), 1);
    assert!(
        out.op
            .plan
            .items
            .iter()
            .all(|item| !short(&item.target).starts_with("/r"))
    );
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/new.txt"));
    assert_eq!(read(&vfs, "/l/up.txt").unwrap(), b"newer on the right");
    // A mirror onto a read-only side deletes nothing there.
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    file(&vfs, "/r/z.txt", b"z", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let mut req = request(&options);
    req.read_only = ReadOnlySides {
        left: false,
        right: true,
    };
    let out = plan(&vfs, &cmp, &req);
    assert!(out.op.plan.items.is_empty());
    assert_eq!(out.counts.delete_right, 0);
    execute(&vfs, &out);
    assert!(exists(&vfs, "/r/z.txt"));
}

#[test]
fn a6_27_rule6_below_a_followed_folder_nothing_is_written() {
    let vfs = memory(CaseSensitivity::Sensitive);
    dir(&vfs, "/outside");
    file(&vfs, "/outside/kept.txt", b"outside", 10);
    file(&vfs, "/outside/stale.txt", b"outside too", 10);
    dir(&vfs, "/l/d");
    file(&vfs, "/l/d/kept.txt", b"left version", 20);
    vfs.insert_symlink(&host("/r/d"), &host("/outside"), meta(1))
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.follow_symlinks = true;
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    for path in ["d/kept.txt", "d/stale.txt"] {
        assert_eq!(
            row(&cmp, &out, path).reason,
            Some(LeaveReason::FollowedLink),
            "{path}"
        );
    }
    execute(&vfs, &out);
    assert_eq!(read(&vfs, "/outside/kept.txt").unwrap(), b"outside");
    assert_eq!(read(&vfs, "/outside/stale.txt").unwrap(), b"outside too");
    // A followed folder on one side only is its link: a mirror removes the link, never what it leads to.
    let vfs = memory(CaseSensitivity::Sensitive);
    dir(&vfs, "/outside");
    file(&vfs, "/outside/x.txt", b"outside", 10);
    file(&vfs, "/l/a.txt", b"a", 10);
    vfs.insert_symlink(&host("/r/link"), &host("/outside"), meta(1))
        .unwrap();
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let link = row(&cmp, &out, "link/");
    assert_eq!(link.action, SyncAction::DeleteRight);
    assert_eq!(
        row(&cmp, &out, "link/x.txt").reason,
        Some(LeaveReason::FollowedLink)
    );
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/link"));
    assert_eq!(read(&vfs, "/outside/x.txt").unwrap(), b"outside");
}

#[test]
fn a6_27_a_folder_whose_last_row_is_a_followed_link_goes_with_it() {
    // A right-only folder holding a file and a link the comparison follows: a mirror removes the file, the link (not what
    // it leads to) and then the folder, as its row says.
    let vfs = memory(CaseSensitivity::Sensitive);
    dir(&vfs, "/outside");
    file(&vfs, "/outside/x.txt", b"outside", 10);
    file(&vfs, "/l/a.txt", b"a", 10);
    dir(&vfs, "/r/w");
    file(&vfs, "/r/w/f.txt", b"f", 10);
    vfs.insert_symlink(&host("/r/w/link"), &host("/outside"), meta(1))
        .unwrap();
    let mut rules = FolderRules::default();
    rules.handling.follow_symlinks = true;
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(row(&cmp, &out, "w/").action, SyncAction::DeleteRight);
    assert!(
        items(&out).contains(&"remove_dir /r/w".to_owned()),
        "{:?}",
        items(&out)
    );
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/w"));
    assert_eq!(read(&vfs, "/outside/x.txt").unwrap(), b"outside");
}

// ---- conflicts -----------------------------------------------------------------------------------------------------

#[test]
fn a6_27_conflicts_are_different_rows_left_by_the_matrix() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/c.txt", b"one", 10);
    file(&vfs, "/r/c.txt", b"other", 10);
    file(&vfs, "/l/o.txt", b"one", 10);
    file(&vfs, "/r/o.txt", b"other", 10);
    let mut options = preset(SyncPreset::UpdateRight);
    options.overrides.insert("o.txt".into(), SyncAction::Leave);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert!(row(&cmp, &out, "c.txt").conflict);
    // An override decided: no conflict.
    assert!(!row(&cmp, &out, "o.txt").conflict);
    assert_eq!(out.counts.conflicts, 1);
    // A mirror says who wins.
    let options = preset(SyncPreset::MirrorRight);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(out.counts.conflicts, 0);
}

// ---- folders -------------------------------------------------------------------------------------------------------

#[test]
fn a6_27_empty_folders_follow_create_empty_both_ways() {
    for create_empty in [false, true] {
        let vfs = memory(CaseSensitivity::Sensitive);
        file(&vfs, "/l/a.txt", b"a", 10);
        dir(&vfs, "/l/new");
        dir(&vfs, "/r/old");
        let mut options = preset(SyncPreset::MirrorRight);
        options.create_empty = create_empty;
        let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
        let out = plan(&vfs, &cmp, &request(&options));
        let (new, old) = (row(&cmp, &out, "new/"), row(&cmp, &out, "old/"));
        assert_eq!(new.situation, Some(Situation::AddedLeft));
        if create_empty {
            assert_eq!(
                (new.action, old.action),
                (SyncAction::CopyRight, SyncAction::DeleteRight)
            );
        } else {
            assert_eq!(
                (new.reason, old.reason),
                (Some(LeaveReason::Matrix), Some(LeaveReason::Matrix))
            );
        }
        assert_clean(&execute(&vfs, &out));
        assert_eq!(exists(&vfs, "/r/new"), create_empty);
        assert_eq!(exists(&vfs, "/r/old"), !create_empty);
        // Mirror to Left takes them the other way.
        let vfs = memory(CaseSensitivity::Sensitive);
        file(&vfs, "/r/a.txt", b"a", 10);
        dir(&vfs, "/r/new");
        dir(&vfs, "/l/old");
        let mut options = preset(SyncPreset::MirrorLeft);
        options.create_empty = create_empty;
        let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
        let out = plan(&vfs, &cmp, &request(&options));
        assert_clean(&execute(&vfs, &out));
        assert_eq!(exists(&vfs, "/l/new"), create_empty);
        assert_eq!(exists(&vfs, "/l/old"), !create_empty);
    }
}

#[test]
fn a6_27_a_wholly_deleted_folder_goes_and_one_keeping_a_filtered_row_stays() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    dir(&vfs, "/r/w");
    dir(&vfs, "/r/w/deep");
    file(&vfs, "/r/w/deep/x.txt", b"x", 10);
    file(&vfs, "/r/w/y.txt", b"y", 10);
    dir(&vfs, "/r/k");
    file(&vfs, "/r/k/a.txt", b"shown", 10);
    file(&vfs, "/r/k/b.log", b"hidden by the filter", 10);
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some("*.txt".into()),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(row(&cmp, &out, "w/").action, SyncAction::DeleteRight);
    assert_eq!(row(&cmp, &out, "w/deep/").action, SyncAction::DeleteRight);
    let k = row(&cmp, &out, "k/");
    assert_eq!(
        (k.action, k.reason),
        (SyncAction::Leave, Some(LeaveReason::Filtered))
    );
    let lines = items(&out);
    assert!(lines.contains(&"remove_dir /r/w".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"remove_dir /r/w/deep".to_owned()),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("remove_dir /r/k")),
        "{lines:?}"
    );
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/w"));
    assert!(!exists(&vfs, "/r/k/a.txt"));
    assert_eq!(read(&vfs, "/r/k/b.log").unwrap(), b"hidden by the filter");
}

#[test]
fn a6_27_a_wholly_deleted_folder_is_one_trash_with_the_bin() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    dir(&vfs, "/r/w");
    file(&vfs, "/r/w/x.txt", b"x", 10);
    file(&vfs, "/r/w/y.txt", b"y", 10);
    file(&vfs, "/r/loose.txt", b"z", 10);
    let mut options = preset(SyncPreset::MirrorRight);
    options.recycle = true;
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let lines = items(&out);
    let trashes: Vec<_> = lines.iter().filter(|l| l.starts_with("trash")).collect();
    assert_eq!(trashes, ["trash /r/w", "trash /r/loose.txt"], "{lines:?}");
    assert_clean(&execute(&vfs, &out));
    let trashed = vfs.trashed();
    assert_eq!(trashed.len(), 2, "{trashed:?}");
    assert!(!exists(&vfs, "/r/w"));
}

/// One `trash` of a whole folder takes what lies below it: right before it acts, that must be what the plan saw. A file
/// added, rewritten, removed or a folder added since: the item fails "changed since the plan was made" and nothing moves.
#[test]
fn a6_27_a_whole_trash_checks_what_lies_below_it_before_it_moves() {
    type Change = fn(&MemoryVfs);
    let changes: [(&str, Change); 6] = [
        ("unchanged", |_| {}),
        ("a file added", |vfs| file(vfs, "/r/w/new.txt", b"precious", 30)),
        ("a file rewritten", |vfs| {
            file(vfs, "/r/w/sub/x.txt", b"rewritten since the plan", 30);
        }),
        ("a file touched", |vfs| file(vfs, "/r/w/y.txt", b"y", 30)),
        ("a file gone", |vfs| vfs.remove(&mspec("/r/w/y.txt")).unwrap()),
        ("a folder added", |vfs| dir(vfs, "/r/w/sub/new")),
    ];
    for fallback in [false, true] {
        for (what, change) in changes {
            let vfs = memory(CaseSensitivity::Sensitive);
            file(&vfs, "/l/a.txt", b"a", 10);
            dir(&vfs, "/r/w/sub");
            file(&vfs, "/r/w/sub/x.txt", b"x", 10);
            file(&vfs, "/r/w/y.txt", b"y", 10);
            let mut options = preset(SyncPreset::MirrorRight);
            options.recycle = true;
            options.recycle_fallback = fallback;
            let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
            let out = plan(&vfs, &cmp, &request(&options));
            let trash = out
                .op
                .plan
                .items
                .iter()
                .find(|item| item.action == Action::Trash)
                .unwrap();
            assert_eq!(short(&trash.target), "/r/w");
            assert!(trash.verify_below, "{what}");
            assert_eq!(trash.fallback.len(), 4, "{what}: {:?}", trash.fallback);
            change(&vfs);
            let below = |vfs: &MemoryVfs| {
                let w = host("/r/w");
                let mut dump = vfs.dump();
                dump.retain(|entry| entry.path.starts_with(&w));
                dump
            };
            let before = below(&vfs);
            let report = execute(&vfs, &out);
            let result = report
                .plan
                .items
                .iter()
                .zip(&report.results)
                .find(|(item, _)| item.action == Action::Trash)
                .map(|(_, result)| result.clone())
                .unwrap();
            if what == "unchanged" {
                assert_eq!(result.status, ItemStatus::Done, "{fallback} {result:?}");
                assert!(!exists(&vfs, "/r/w"));
                assert_eq!(vfs.trashed().len(), 1);
                continue;
            }
            assert_eq!(result.status, ItemStatus::Failed, "{what} {fallback}");
            let error = result.error.unwrap();
            assert!(
                error.message.contains("changed since the plan was made"),
                "{what}: {error:?}"
            );
            assert!(vfs.trashed().is_empty(), "{what}: {:?}", vfs.trashed());
            assert_eq!(below(&vfs), before, "{what}: nothing moves");
        }
    }
}

// ---- order ---------------------------------------------------------------------------------------------------------

#[test]
fn a6_27_deletes_then_right_then_left_and_a_file_replaces_a_folder_of_its_name() {
    let vfs = memory(CaseSensitivity::Sensitive);
    // `x` is a file on the left and a folder on the right: two rows (D-20).
    file(&vfs, "/l/x", b"the file", 10);
    dir(&vfs, "/r/x");
    file(&vfs, "/r/x/inner.txt", b"inner", 10);
    // `y` the other way round.
    dir(&vfs, "/l/y");
    file(&vfs, "/l/y/in.txt", b"in", 10);
    file(&vfs, "/r/y", b"a file y", 10);
    file(&vfs, "/l/n.txt", b"newer", 20);
    file(&vfs, "/r/n.txt", b"older", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let actions: Vec<Action> = out.op.plan.items.iter().map(|i| i.action).collect();
    let first_copy = actions
        .iter()
        .position(|a| matches!(a, Action::Copy | Action::CreateDir))
        .unwrap();
    assert!(
        actions[..first_copy]
            .iter()
            .all(|a| matches!(a, Action::Remove | Action::RemoveDir)),
        "{:?}",
        items(&out)
    );
    assert!(
        actions[first_copy..]
            .iter()
            .all(|a| !matches!(a, Action::Remove | Action::RemoveDir))
    );
    // No item expects what the deletes take away.
    for item in &out.op.plan.items[first_copy..] {
        assert!(
            !item.replaces || short(&item.target) == "/r/n.txt",
            "{:?}",
            items(&out)
        );
    }
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/x").unwrap(), b"the file");
    assert_eq!(read(&vfs, "/r/y/in.txt").unwrap(), b"in");
    assert_eq!(read(&vfs, "/r/n.txt").unwrap(), b"newer");
    // Update Both: right before left.
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    file(&vfs, "/r/b.txt", b"b", 10);
    let options = preset(SyncPreset::UpdateBoth);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(items(&out), ["copy /r/a.txt", "copy /l/b.txt"]);
}

#[test]
fn a6_27_update_replaces_only_a_rows_own_counterpart() {
    // A left folder `y` and a right file `y` are two rows. Update Right leaves the right file (Update never removes), so
    // the folder cannot be made there: the item fails, and the file stays.
    let vfs = memory(CaseSensitivity::Sensitive);
    dir(&vfs, "/l/y");
    file(&vfs, "/l/y/in.txt", b"in", 10);
    file(&vfs, "/r/y", b"a right file", 99);
    file(&vfs, "/l/x", b"a left file", 10);
    dir(&vfs, "/r/x");
    file(&vfs, "/r/x/keep.txt", b"keep", 10);
    let options = preset(SyncPreset::UpdateRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert!(
        out.op.plan.items.iter().all(|item| !item.replaces),
        "{:?}",
        items(&out)
    );
    let report = execute(&vfs, &out);
    assert!(!report.ok());
    assert_eq!(read(&vfs, "/r/y").unwrap(), b"a right file");
    assert_eq!(read(&vfs, "/r/x/keep.txt").unwrap(), b"keep");
    // A case-sensitive comparison on a file system that folds: `b.txt` must not overwrite `B.txt`, which Update keeps.
    let vfs = memory(CaseSensitivity::Insensitive);
    file(&vfs, "/l/b.txt", b"lower", 20);
    file(&vfs, "/r/B.txt", b"upper", 10);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Sensitive;
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert!(out.op.plan.items.iter().all(|item| !item.replaces));
    execute(&vfs, &out);
    assert_eq!(read(&vfs, "/r/B.txt").unwrap(), b"upper");
}

#[test]
fn a6_27_case_only_names() {
    // A case-insensitive comparison pairs `A.txt` with `a.txt`: the right keeps its name and takes the content.
    let vfs = memory(CaseSensitivity::Insensitive);
    file(&vfs, "/l/A.txt", b"left content", 20);
    file(&vfs, "/r/a.txt", b"right", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(out.counts.copy_right, 1);
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/a.txt").unwrap(), b"left content");
    // A case-sensitive comparison on a file system that folds: `b.txt` and `B.txt` are two rows, and the delete frees the
    // name the copy takes.
    let vfs = memory(CaseSensitivity::Insensitive);
    file(&vfs, "/l/b.txt", b"lower", 20);
    file(&vfs, "/r/B.txt", b"upper", 10);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Sensitive;
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_eq!(out.counts.delete_right, 1);
    assert_eq!(out.counts.copy_right, 1);
    assert_clean(&execute(&vfs, &out));
    let names: Vec<String> = vfs
        .dump()
        .into_iter()
        .filter(|e| e.kind == EntryKind::File && e.path.replace('\\', "/").contains("/r/"))
        .map(|e| e.path.replace('\\', "/"))
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with("/r/b.txt"), "{names:?}");
    assert_eq!(read(&vfs, "/r/b.txt").unwrap(), b"lower");
}

/// With `filename_case` a case-only difference is a difference: a sync gives the counterpart the source's spelling too (one
/// `rename` after the copy), so a second run plans nothing. On a file system that folds and on one that does not.
#[test]
fn a6_27_with_filename_case_the_target_takes_the_sources_spelling() {
    for case in [CaseSensitivity::Insensitive, CaseSensitivity::Sensitive] {
        let vfs = memory(case);
        file(&vfs, "/l/a.txt", b"left content", 20);
        file(&vfs, "/r/A.txt", b"right", 10);
        let mut rules = FolderRules::default();
        rules.alignment.case = CaseMode::Insensitive;
        rules.criteria.filename_case = true;
        let options = preset(SyncPreset::MirrorRight);
        let cmp = sync_compare(&vfs, &rules, &options);
        let out = plan(&vfs, &cmp, &request(&options));
        assert_eq!(out.counts.copy_right, 1, "{case:?}");
        let lines = items(&out);
        assert!(
            lines.iter().any(|l| l == "rename /r/a.txt"),
            "{case:?} {lines:?}"
        );
        assert_clean(&execute(&vfs, &out));
        let names: Vec<String> = vfs
            .dump()
            .into_iter()
            .filter(|e| e.kind == EntryKind::File && e.path.replace('\\', "/").contains("/r/"))
            .map(|e| e.path.replace('\\', "/"))
            .collect();
        assert_eq!(names.len(), 1, "{case:?} {names:?}");
        assert!(names[0].ends_with("/r/a.txt"), "{case:?} {names:?}");
        assert_eq!(read(&vfs, "/r/a.txt").unwrap(), b"left content");
        let cmp = sync_compare(&vfs, &rules, &options);
        let again = plan(&vfs, &cmp, &request(&options));
        assert!(again.op.plan.items.is_empty(), "{case:?} {:?}", items(&again));
        // Without `filename_case` the spelling is no difference, and nothing is renamed.
        let vfs = memory(case);
        file(&vfs, "/l/a.txt", b"left content", 20);
        file(&vfs, "/r/A.txt", b"right", 10);
        rules.criteria.filename_case = false;
        let cmp = sync_compare(&vfs, &rules, &options);
        let out = plan(&vfs, &cmp, &request(&options));
        assert!(!items(&out).iter().any(|l| l.starts_with("rename")), "{:?}", items(&out));
    }
}

/// Two source names of one key under a case-insensitive comparison: the one the other side has no entry for is left at
/// planning (`ambiguous_name`, counted), with no item; the pair is synced. Below a folder that is such a name, every row is
/// left too. A second run plans the same: nothing.
#[test]
fn a6_27_two_source_names_of_one_key_are_left_as_ambiguous() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"lower", 20);
    file(&vfs, "/l/A.TXT", b"UPPER", 20);
    file(&vfs, "/r/a.txt", b"r", 10);
    file(&vfs, "/l/Sub/x", b"x", 20);
    file(&vfs, "/l/SUB/y", b"y", 20);
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Insensitive;
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &rules, &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let read_ = cmp.read();
    let ambiguous: Vec<String> = out
        .rows
        .iter()
        .filter(|r| r.reason == Some(LeaveReason::AmbiguousName))
        .map(|r| read_.path(r.node).as_str().to_owned())
        .collect();
    assert_eq!(
        ambiguous.iter().filter(|p| p.as_str() == "A.TXT").count(),
        1,
        "{ambiguous:?}"
    );
    assert!(ambiguous.iter().any(|p| p == "Sub/x" || p == "SUB/y"), "{ambiguous:?}");
    assert_eq!(out.counts.ambiguous as usize, ambiguous.len());
    assert_eq!(row(&cmp, &out, "a.txt").action, SyncAction::CopyRight);
    let lines = items(&out);
    assert!(!lines.iter().any(|l| l.contains("A.TXT")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.starts_with("skip")), "{lines:?}");
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/r/a.txt").unwrap(), b"lower");
    assert!(!exists(&vfs, "/r/A.TXT"));
    let cmp = sync_compare(&vfs, &rules, &options);
    let again = plan(&vfs, &cmp, &request(&options));
    assert!(!items(&again).iter().any(|l| l.contains("a.txt")), "{:?}", items(&again));
    assert_eq!(again.counts.ambiguous, out.counts.ambiguous);
}

#[test]
fn a6_27_links_are_copied_and_deleted_as_links() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/target.txt", b"target", 10);
    vfs.insert_symlink(&host("/l/ln"), "../target.txt", meta(10))
        .unwrap();
    vfs.insert_symlink(&host("/r/old"), &host("/target.txt"), meta(10))
        .unwrap();
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_clean(&execute(&vfs, &out));
    let dump = vfs.dump();
    let ln = dump.iter().find(|e| e.path == host("/r/ln")).unwrap();
    assert_eq!(ln.kind, EntryKind::Symlink);
    assert!(!exists(&vfs, "/r/old"));
    assert_eq!(read(&vfs, "/target.txt").unwrap(), b"target");
}

// ---- effective rules -----------------------------------------------------------------------------------------------

#[test]
fn a6_27_effective_rules() {
    let mut rules = FolderRules::default();
    rules.handling.flatten = true;
    rules.handling.auto_scan_subfolders = false;
    rules.handling.scan_orphan_folders = false;
    let options = preset(SyncPreset::MirrorRight);
    let err = sync_folder_rules(&rules, &options).unwrap_err();
    assert!(err.to_string().contains("folder structure"), "{err}");
    rules.handling.flatten = false;
    let effective = sync_folder_rules(&rules, &options).unwrap();
    assert!(effective.handling.auto_scan_subfolders && effective.handling.scan_orphan_folders);
    assert_eq!(effective.handling.archives, ArchiveHandling::AsFiles);
    let within = SyncOptions {
        within_archives: true,
        ..options.clone()
    };
    assert_eq!(
        sync_folder_rules(&rules, &within)
            .unwrap()
            .handling
            .archives,
        ArchiveHandling::Always
    );
    // A flat model is refused by the plan too.
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/a.txt", b"a", 10);
    rules.handling.flatten = true;
    let cmp = compared(&vfs, &rules);
    let err = plan_sync(&cmp.read(), &vfs, &request(&options), &Ctl::detached()).unwrap_err();
    assert!(matches!(err, FolderError::InvalidRules(_)));
}

// ---- refusals ------------------------------------------------------------------------------------------------------

#[test]
fn a6_27_an_empty_source_is_refused_unless_allowed() {
    let setup = || {
        let vfs = memory(CaseSensitivity::Sensitive);
        file(&vfs, "/r/a.txt", b"a", 10);
        dir(&vfs, "/r/d");
        file(&vfs, "/r/d/b.txt", b"b", 10);
        vfs
    };
    let vfs = setup();
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let err = plan_sync(&cmp.read(), &vfs, &request(&options), &Ctl::detached()).unwrap_err();
    assert!(
        matches!(err, FolderError::EmptySource { side: Side::Left }),
        "{err:?}"
    );
    assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::InvalidInput);
    // Mirror to Left from an empty right: the same, the other way.
    let empty_right = memory(CaseSensitivity::Sensitive);
    file(&empty_right, "/l/a.txt", b"a", 10);
    let left = preset(SyncPreset::MirrorLeft);
    let cmp2 = sync_compare(&empty_right, &FolderRules::default(), &left);
    let err = plan_sync(
        &cmp2.read(),
        &empty_right,
        &request(&left),
        &Ctl::detached(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        FolderError::EmptySource { side: Side::Right }
    ));
    // Allowed: it deletes.
    let mut req = request(&options);
    req.allow_empty_source = true;
    let out = plan(&vfs, &cmp, &req);
    assert_clean(&execute(&vfs, &out));
    assert!(!exists(&vfs, "/r/a.txt") && !exists(&vfs, "/r/d"));
    // Update from an empty base deletes nothing: no refusal.
    let vfs = setup();
    for which in [
        SyncPreset::UpdateRight,
        SyncPreset::UpdateLeft,
        SyncPreset::UpdateBoth,
    ] {
        let options = preset(which);
        let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
        plan(&vfs, &cmp, &request(&options));
    }
    let options = preset(SyncPreset::UpdateLeft);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    assert_clean(&execute(&vfs, &out));
    assert_eq!(read(&vfs, "/l/d/b.txt").unwrap(), b"b");
}

#[test]
fn a6_27_a_comparison_that_did_not_finish_deletes_nothing() {
    let vfs = memory(CaseSensitivity::Sensitive);
    for name in ["a", "b", "c"] {
        dir(&vfs, &format!("/l/{name}"));
        file(&vfs, &format!("/l/{name}/x.txt"), b"x", 10);
        dir(&vfs, &format!("/r/{name}"));
        file(&vfs, &format!("/r/{name}/stale.txt"), b"stale", 10);
    }
    let options = preset(SyncPreset::MirrorRight);
    let rules = sync_folder_rules(&FolderRules::default(), &options).unwrap();
    let mut refused = 0;
    for after in 1..=3 {
        let cancel = CancelToken::new();
        let observer = CancelAfter {
            after,
            seen: AtomicUsize::new(0),
            cancel: &cancel,
        };
        let cmp = run_on(
            &vfs,
            &rules,
            None,
            &engines(1 << 20),
            &default_now(),
            &observer,
            &cancel,
            false,
        )
        .unwrap();
        assert!(!cmp.is_complete(), "after {after}");
        match plan_sync(&cmp.read(), &vfs, &request(&options), &Ctl::detached()) {
            Err(err) => {
                assert!(err.to_string().contains("not complete"), "{err}");
                refused += 1;
            }
            // Nothing scanned that could be deleted: what was not reached is `compare_error`.
            Ok(out) => {
                assert_eq!(out.counts.delete_right, 0, "after {after}");
                assert!(out.counts.unresolved > 0);
            }
        }
        // A plan without deletes still plans.
        let update = preset(SyncPreset::UpdateRight);
        let out = plan(&vfs, &cmp, &request(&update));
        assert_eq!(out.counts.delete_right, 0);
    }
    assert!(refused > 0, "no case reached a delete before the stop");
    for name in ["a", "b", "c"] {
        assert!(exists(&vfs, &format!("/r/{name}/stale.txt")));
    }
}

// ---- the folder summaries ------------------------------------------------------------------------------------------

#[test]
fn a6_27_folder_summaries_equal_a_recount() {
    let vfs = memory(CaseSensitivity::Sensitive);
    for (path, bytes, secs) in [
        ("/l/a/x.txt", "xx", 20),
        ("/r/a/x.txt", "x", 10),
        ("/l/a/b/y.txt", "yyy", 10),
        ("/r/a/b/z.txt", "z", 10),
        ("/r/a/b/c/w.txt", "w", 10),
        ("/l/top.txt", "t", 10),
        ("/r/q.txt", "q", 10),
        ("/l/s.txt", "same", 10),
        ("/r/s.txt", "same", 10),
    ] {
        let parent = path.rsplit_once('/').unwrap().0;
        let _ = vfs.insert_dir(&host(parent), meta(1));
        let mut at = String::new();
        for part in parent.split('/').filter(|p| !p.is_empty()) {
            at.push('/');
            at.push_str(part);
            let _ = vfs.insert_dir(&host(&at), meta(1));
        }
        file(&vfs, path, bytes.as_bytes(), secs);
    }
    for which in [
        SyncPreset::MirrorRight,
        SyncPreset::UpdateBoth,
        SyncPreset::MirrorLeft,
    ] {
        let options = preset(which);
        let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
        let out = plan(&vfs, &cmp, &request(&options));
        let read = cmp.read();
        let rows: BTreeMap<NodeId, SyncRow> = out.rows.iter().map(|r| (r.node, *r)).collect();
        assert_eq!(
            rows.len() as u32,
            read.len() - 1,
            "one row per row of the tree"
        );
        let recount = |folder: NodeId| {
            let mut total = SyncCounts::default();
            let mut stack: Vec<NodeId> = read.children(folder).collect();
            while let Some(node) = stack.pop() {
                let r = rows[&node];
                let bytes = match r.action.reads() {
                    Some(side) if !read.node(node).unwrap().is_folder() => {
                        read.node(node).unwrap().side(side).unwrap().size
                    }
                    _ => 0,
                };
                total.add(&SyncCounts::of_row(&r, bytes));
                stack.extend(read.children(node));
            }
            total
        };
        for (node, _) in read.preorder() {
            if read.node(node).unwrap().is_folder() {
                assert_eq!(
                    out.folder(node),
                    recount(node),
                    "{which:?} {}",
                    read.path(node).as_str()
                );
            }
        }
        assert_eq!(out.folder(NodeId::ROOT), out.counts);
        assert_eq!(out.counts, recount(NodeId::ROOT));
    }
}

// ---- archives ------------------------------------------------------------------------------------------------------

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

fn gz_of(data: &[u8]) -> Vec<u8> {
    // A stored deflate block: a gzip any reader takes.
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.push(1);
    let len = u16::try_from(data.len()).unwrap();
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

struct World {
    mem: Arc<MemoryVfs>,
    vfs: ArchiveVfs,
    _dir: tempfile::TempDir,
}

fn world() -> World {
    let mem = Arc::new(memory(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let masks = ArchiveMasks::new([
        (ArchiveFormat::Tar, "*.tar".to_owned()),
        (ArchiveFormat::Gz, "*.gz".to_owned()),
    ])
    .unwrap();
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
    fn compared(&self, rules: &FolderRules) -> FolderComparison {
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

    fn member(&self, archive: &str, member: &str) -> Option<Vec<u8>> {
        let spec = Spec::archive_member(mspec(archive), member);
        let mut reader = self.vfs.open_read(&spec).ok()?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).ok()?;
        Some(out)
    }
}

#[test]
fn a6_27_within_archives_both_ways_and_one_rebuild_and_backup_per_archive() {
    let setup = || {
        let w = world();
        w.mem
            .insert_file(
                &host("/l/a.tar"),
                tar_of(&[("x", b"new x"), ("y", b"new y")]),
                meta(20),
            )
            .unwrap();
        w.mem
            .insert_file(
                &host("/r/a.tar"),
                tar_of(&[("x", b"old"), ("y", b"old"), ("z", b"stale")]),
                meta(10),
            )
            .unwrap();
        w
    };
    // Within archives: member by member, one rebuild and one backup of the archive.
    let w = setup();
    let mut options = preset(SyncPreset::MirrorRight);
    options.within_archives = true;
    let cmp = w.compared(&sync_folder_rules(&FolderRules::default(), &options).unwrap());
    let mut req = request(&options);
    req.settings.backup = Some(BackupPolicy {
        style: BackupStyle::NameBak,
        folder: None,
    });
    let out = plan_sync(&cmp.read(), &w.vfs, &req, &Ctl::detached()).unwrap();
    assert_eq!(out.counts.copy_right, 2);
    assert_eq!(out.counts.delete_right, 1);
    assert_eq!(out.op.exec.archive_backups.len(), 1);
    let before = w.vfs.counters().rebuilds;
    assert_clean(&execute(&w.vfs, &out));
    assert_eq!(
        w.vfs.counters().rebuilds - before,
        1,
        "one rebuild per archive"
    );
    assert_eq!(w.member("/r/a.tar", "x").unwrap(), b"new x");
    assert!(w.member("/r/a.tar", "z").is_none());
    let backup = out.op.exec.archive_backups.values().next().unwrap();
    assert!(w.mem.read_back(&backup.canonical()).is_some());
    // Without: the archive is a file, copied whole.
    let w = setup();
    let options = preset(SyncPreset::MirrorRight);
    let cmp = w.compared(&sync_folder_rules(&FolderRules::default(), &options).unwrap());
    let out = plan_sync(&cmp.read(), &w.vfs, &request(&options), &Ctl::detached()).unwrap();
    assert_eq!(items(&out), ["copy /r/a.tar"]);
    assert_clean(&execute(&w.vfs, &out));
    assert_eq!(read(&w.mem, "/r/a.tar"), read(&w.mem, "/l/a.tar"));
}

#[test]
fn a6_27_a_single_stream_cannot_take_a_member() {
    let w = world();
    w.mem
        .insert_file(&host("/l/d.txt.gz"), gz_of(b"left stream"), meta(20))
        .unwrap();
    w.mem
        .insert_file(&host("/r/d.txt.gz"), gz_of(b"right"), meta(10))
        .unwrap();
    let before = read(&w.mem, "/r/d.txt.gz");
    let mut options = preset(SyncPreset::MirrorRight);
    options.within_archives = true;
    let cmp = w.compared(&sync_folder_rules(&FolderRules::default(), &options).unwrap());
    let out = plan_sync(&cmp.read(), &w.vfs, &request(&options), &Ctl::detached()).unwrap();
    let member = row(&cmp, &out, "d.txt.gz/d.txt");
    assert_eq!(
        (member.action, member.reason),
        (SyncAction::Leave, Some(LeaveReason::Capability))
    );
    assert!(out.op.plan.items.is_empty(), "{:?}", items(&out));
    assert!(
        out.op
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::CapabilityMissing)
    );
    execute(&w.vfs, &out);
    assert_eq!(read(&w.mem, "/r/d.txt.gz"), before);
}

// ---- a tree that changed since the comparison ----------------------------------------------------------------------

#[test]
fn a6_27_a_tree_changed_since_the_comparison_keeps_what_changed() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/n.txt", b"left newer", 20);
    file(&vfs, "/r/n.txt", b"right older", 10);
    file(&vfs, "/r/orphan.txt", b"orphan", 10);
    dir(&vfs, "/r/w");
    file(&vfs, "/r/w/x.txt", b"x", 10);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    // Someone writes on the right after the comparison.
    file(&vfs, "/r/n.txt", b"edited since", 30);
    file(&vfs, "/r/orphan.txt", b"edited too", 30);
    file(&vfs, "/r/w/new.txt", b"added since", 30);
    let report = execute(&vfs, &out);
    let failed: Vec<String> = report
        .plan
        .items
        .iter()
        .zip(&report.results)
        .filter(|(_, r)| r.status == ItemStatus::Failed)
        .map(|(item, _)| short(&item.target))
        .collect();
    assert!(failed.contains(&"/r/n.txt".to_owned()), "{failed:?}");
    assert!(failed.contains(&"/r/orphan.txt".to_owned()), "{failed:?}");
    assert!(failed.contains(&"/r/w".to_owned()), "{failed:?}");
    assert_eq!(read(&vfs, "/r/n.txt").unwrap(), b"edited since");
    assert_eq!(read(&vfs, "/r/orphan.txt").unwrap(), b"edited too");
    assert_eq!(read(&vfs, "/r/w/new.txt").unwrap(), b"added since");
}

// ---- confirmation --------------------------------------------------------------------------------------------------

#[test]
fn a6_27_a_sync_asks_its_question_and_a_mirror_over_a_newer_file() {
    let vfs = memory(CaseSensitivity::Sensitive);
    file(&vfs, "/l/n.txt", b"left older", 10);
    file(&vfs, "/r/n.txt", b"right newer", 20);
    let options = preset(SyncPreset::MirrorRight);
    let cmp = sync_compare(&vfs, &FolderRules::default(), &options);
    let out = plan(&vfs, &cmp, &request(&options));
    let mut machine = Confirmation::new(&out.op, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::Operation);
    machine.reply(hikasa_folder::ops::Reply::Yes);
    assert_eq!(machine.next().unwrap().kind, QuestionKind::OverwriteNewer);
    // `no-to-all` keeps the newer file.
    let declined = Confirmation::new(&out.op, &ConfirmPolicy::ALL, ConfirmMode::NoToAll)
        .finish(out.op.clone())
        .unwrap();
    assert_eq!(declined.plan.items[0].skip, Some(SkipReason::Declined));
    ops::execute_with(&vfs, &vfs, declined.plan, &declined.exec, &Ctl::detached());
    assert_eq!(read(&vfs, "/r/n.txt").unwrap(), b"right newer");
    let _ = SyncMatrix::default();
    let _ = OrphanReading::Added;
}
