//! M2 W3 acceptance A3.6: the nine symbolic-link rules of contract M2 §1.3.3 (D-94), each on `MemoryVfs` and on
//! a local tempdir (Unix; on Windows the local half runs only when creating links is permitted, else it is
//! skipped with a message). Trash of a link is checked on `MemoryVfs` only (the real recycle bin is exercised
//! in an isolated subprocess by tests/trash.rs).

mod common;

use std::io::Write as _;

use common::{Fx, ft};
use hikasa_core::{CaseSensitivity, Ctl, ErrorKind, Spec, WarningCode};
use hikasa_vfs::ops::{
    self, Action, CopyOptions, ItemStatus, LinkPolicy, MoveOptions, OpReport, OverwritePolicy,
    RemoveOptions, SkipReason,
};
use hikasa_vfs::{
    AttrChange, Attrs, Capabilities, EntryKind, SetTimes, VfsError, VfsOp, WriteOptions,
};

/// The fixtures of every rule: `MemoryVfs` and a local tempdir. Each gets `outside/keep`, `pointed` (a file),
/// `lpointed` → `pointed`, `loutside` → `outside`; `None` when links cannot be created here.
fn fixtures() -> Vec<Fx> {
    [Fx::mem(CaseSensitivity::Sensitive), Fx::local()]
        .into_iter()
        .filter(|fx| {
            fx.file("outside/keep", b"keep");
            fx.file("pointed", b"pointed bytes");
            fx.link("lpointed", "pointed", false) && fx.link("loutside", "outside", true)
        })
        .collect()
}

fn copy_opts(overwrite: OverwritePolicy) -> CopyOptions {
    CopyOptions {
        recursive: true,
        overwrite,
        ..CopyOptions::default()
    }
}

fn copy(fx: &Fx, pairs: &[(&str, &str)], opts: &CopyOptions) -> OpReport {
    let pairs: Vec<(Spec, Spec)> = pairs
        .iter()
        .map(|(a, b)| (fx.spec(a), fx.spec(b)))
        .collect();
    let plan = ops::plan_copy(fx.vfs(), fx.vfs(), &pairs, opts, &Ctl::detached()).unwrap();
    ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached())
}

fn statuses(report: &OpReport) -> Vec<(Action, ItemStatus)> {
    report
        .plan
        .items
        .iter()
        .zip(&report.results)
        .map(|(i, r)| (i.action, r.status))
        .collect()
}

/// The pointed-to data is untouched.
fn assert_targets_intact(fx: &Fx, label: &str) {
    assert_eq!(fx.read("pointed"), b"pointed bytes", "{label}: pointed");
    assert_eq!(fx.read("outside/keep"), b"keep", "{label}: outside/keep");
    assert_eq!(fx.kind("outside"), Some(EntryKind::Dir), "{label}");
}

// ---- rule 1: remove, move_to_trash, rename, set_times act on the link itself --------------------------------

#[test]
fn a3_6_rule1_remove_link_to_folder_keeps_the_content() {
    for fx in fixtures() {
        fx.vfs().remove(&fx.spec("loutside")).unwrap();
        fx.vfs().remove(&fx.spec("lpointed")).unwrap();
        assert_eq!(fx.kind("loutside"), None);
        assert_targets_intact(&fx, fx.label());
        // Through ops as well.
        fx.link("l2", "outside", true);
        let plan = ops::plan_remove(
            fx.vfs(),
            &[fx.spec("l2")],
            &RemoveOptions {
                recursive: true,
                trash: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            (plan.items[0].action, plan.items[0].kind),
            (Action::Remove, EntryKind::Symlink)
        );
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert!(report.ok());
        assert_targets_intact(&fx, fx.label());
    }
}

#[test]
fn a3_6_rule1_rename_moves_the_link() {
    for fx in fixtures() {
        fx.dir("sub");
        // A relative target keeps its text (it now resolves from `sub`, where `pointed` is absent).
        fx.vfs()
            .rename(&fx.spec("lpointed"), &fx.spec("sub/moved"), false)
            .unwrap();
        let moved = fx.lstat("sub/moved").unwrap();
        assert_eq!(moved.kind, EntryKind::Symlink, "{}", fx.label());
        assert_eq!(moved.link_target.as_deref(), Some("pointed"));
        assert_eq!(fx.kind("lpointed"), None);
        // A move of a link through ops is one rename of the link.
        let plan = ops::plan_move(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("loutside"), fx.spec("lmoved"))],
            &MoveOptions::default(),
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(
            (plan.items[0].action, plan.items[0].kind),
            (Action::Rename, EntryKind::Symlink)
        );
        assert!(ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached()).ok());
        assert_eq!(fx.kind("lmoved"), Some(EntryKind::Symlink));
        assert_eq!(fx.read("lmoved/keep"), b"keep");
        assert_targets_intact(&fx, fx.label());
    }
}

#[test]
fn a3_6_rule1_trash_moves_the_link_memory() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("outside/keep", b"keep");
    fx.link("loutside", "outside", true);
    let report = {
        let plan = ops::plan_remove(
            fx.vfs(),
            &[fx.spec("loutside")],
            &RemoveOptions {
                recursive: false,
                trash: true,
            },
            &Ctl::detached(),
        )
        .unwrap();
        ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached())
    };
    assert_eq!(statuses(&report), [(Action::Trash, ItemStatus::Done)]);
    assert_eq!(fx.memory().trashed(), [fx.text("loutside")]);
    assert_eq!(fx.kind("loutside"), None);
    assert_eq!(fx.read("outside/keep"), b"keep");
}

#[test]
fn a3_6_rule1_set_times_on_a_link_leaves_the_target() {
    for fx in fixtures() {
        let before = (
            fx.lstat("pointed").unwrap().mtime,
            fx.lstat("outside").unwrap().mtime,
        );
        let times = SetTimes {
            modified: Some(ft(1_234_567_890)),
            created: None,
        };
        for link in ["lpointed", "loutside"] {
            fx.vfs().set_times(&fx.spec(link), &times).unwrap();
            assert_eq!(
                fx.lstat(link).unwrap().mtime,
                times.modified,
                "{} {link}",
                fx.label()
            );
        }
        assert_eq!(
            (
                fx.lstat("pointed").unwrap().mtime,
                fx.lstat("outside").unwrap().mtime
            ),
            before,
            "{}",
            fx.label()
        );
        // touch of a link through ops: the link itself.
        let plan = ops::plan_touch(
            fx.vfs(),
            &[fx.spec("lpointed")],
            &ops::TouchOptions {
                times: SetTimes {
                    modified: Some(ft(1_000_000_000)),
                    created: None,
                },
                create: false,
            },
        );
        assert_eq!(plan.items[0].kind, EntryKind::Symlink);
        assert!(ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached()).ok());
        assert_eq!(fx.lstat("lpointed").unwrap().mtime, Some(ft(1_000_000_000)));
        assert_eq!(fx.lstat("pointed").unwrap().mtime, before.0);
    }
}

// ---- rule 2: set_attrs on a link ---------------------------------------------------------------------------

#[test]
fn a3_6_rule2_set_attrs_on_a_link_is_unsupported() {
    for fx in fixtures() {
        if !fx.is_mem() && cfg!(windows) {
            continue;
        }
        let mode = |rel: &str| match fx.lstat(rel).unwrap().attrs {
            Attrs::Unix { meta } => Some(meta.mode),
            _ => None,
        };
        let before = mode("pointed");
        for link in ["lpointed", "loutside"] {
            let err = fx
                .vfs()
                .set_attrs(&fx.spec(link), &AttrChange::Mode(0o600))
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    VfsError::OperationUnsupported {
                        op: VfsOp::SetAttrs,
                        ..
                    }
                ),
                "{} {link}: {err:?}",
                fx.label()
            );
            assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::Unsupported);
        }
        assert_eq!(mode("pointed"), before, "{}", fx.label());
    }
}

// ---- rule 3: remove_tree never descends a link -------------------------------------------------------------

#[test]
fn a3_6_rule3_remove_tree_leaves_an_outside_folder_intact() {
    for fx in fixtures() {
        fx.file("victim/a", b"a");
        fx.link("victim/out", "../outside", true);
        fx.link("victim/sub/deep", "../../outside", true);
        let plan = ops::plan_remove(
            fx.vfs(),
            &[fx.spec("victim")],
            &RemoveOptions {
                recursive: true,
                trash: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        let listed: Vec<(Action, Spec, EntryKind)> = plan
            .items
            .iter()
            .map(|i| (i.action, i.target.clone(), i.kind))
            .collect();
        assert_eq!(
            listed,
            [
                (Action::Remove, fx.spec("victim/a"), EntryKind::File),
                (Action::Remove, fx.spec("victim/out"), EntryKind::Symlink),
                (
                    Action::Remove,
                    fx.spec("victim/sub/deep"),
                    EntryKind::Symlink
                ),
                (Action::RemoveDir, fx.spec("victim/sub"), EntryKind::Dir),
                (Action::RemoveDir, fx.spec("victim"), EntryKind::Dir),
            ],
            "{}",
            fx.label()
        );
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert!(report.ok(), "{}", fx.label());
        assert_eq!(fx.kind("victim"), None);
        assert_targets_intact(&fx, fx.label());
    }
}

// ---- rule 4: copy or move onto a link replaces the link ----------------------------------------------------

#[test]
fn a3_6_rule4_copy_onto_a_link_replaces_the_link() {
    for fx in fixtures() {
        fx.file("src", b"new content");
        let report = copy(
            &fx,
            &[("src", "lpointed")],
            &copy_opts(OverwritePolicy::Always),
        );
        assert_eq!(
            statuses(&report),
            [(Action::Copy, ItemStatus::Done)],
            "{}",
            fx.label()
        );
        assert!(report.plan.items[0].replaces);
        assert_eq!(fx.kind("lpointed"), Some(EntryKind::File), "{}", fx.label());
        assert_eq!(fx.read("lpointed"), b"new content");
        assert_targets_intact(&fx, fx.label());
        // A link copied onto a link replaces it with a link.
        fx.link("lsrc", "outside", true);
        fx.link("lother", "pointed", false);
        let report = copy(
            &fx,
            &[("lsrc", "lother")],
            &copy_opts(OverwritePolicy::Always),
        );
        assert_eq!(
            statuses(&report),
            [(Action::Link, ItemStatus::Done)],
            "{}",
            fx.label()
        );
        assert_eq!(
            fx.lstat("lother").unwrap().link_target.as_deref(),
            Some("outside")
        );
        assert_targets_intact(&fx, fx.label());
        // Move onto a link (one rename with replace).
        fx.file("msrc", b"moved");
        fx.link("lmove", "pointed", false);
        let plan = ops::plan_move(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("msrc"), fx.spec("lmove"))],
            &MoveOptions {
                overwrite: OverwritePolicy::Always,
                create_parents: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(plan.items[0].action, Action::Rename);
        assert!(ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached()).ok());
        assert_eq!(fx.kind("lmove"), Some(EntryKind::File));
        assert_eq!(fx.read("lmove"), b"moved");
        assert_targets_intact(&fx, fx.label());
    }
}

#[test]
fn a3_6_rule4_move_across_back_ends_onto_a_link_replaces_the_link() {
    let src = Fx::mem(CaseSensitivity::Sensitive);
    let dst = Fx::mem(CaseSensitivity::Sensitive);
    src.file("f", b"moved");
    dst.file("pointed", b"pointed bytes");
    dst.link("l", "pointed", false);
    let plan = ops::plan_move(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("f"), dst.spec("l"))],
        &MoveOptions {
            overwrite: OverwritePolicy::Always,
            create_parents: false,
        },
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(plan.items[0].action, Action::Move);
    assert!(ops::execute(src.vfs(), dst.vfs(), plan, &Ctl::detached()).ok());
    assert_eq!(dst.kind("l"), Some(EntryKind::File));
    assert_eq!(dst.read("l"), b"moved");
    assert_eq!(dst.read("pointed"), b"pointed bytes");
    assert_eq!(src.kind("f"), None);
}

// ---- rule 5: a target subfolder that is a link is never written through -------------------------------------

#[test]
fn a3_6_rule5_target_subfolder_link_is_not_written_through() {
    for policy in [
        OverwritePolicy::Fail,
        OverwritePolicy::Skip,
        OverwritePolicy::Always,
    ] {
        for fx in fixtures() {
            let label = format!("{} {policy:?}", fx.label());
            fx.file("src/d/new", b"new");
            fx.dir("dst");
            fx.link("dst/d", "../outside", true);
            let report = copy(&fx, &[("src", "dst")], &copy_opts(policy));
            // keep_dir dst, then d.
            assert_eq!(report.plan.items[0].action, Action::KeepDir, "{label}");
            let d = (report.plan.items[1].action, report.results[1].status);
            match policy {
                OverwritePolicy::Fail => {
                    assert_eq!(d, (Action::CreateDir, ItemStatus::Failed), "{label}");
                    assert_eq!(
                        report.results[1].error.as_ref().unwrap().kind,
                        ErrorKind::AlreadyExists
                    );
                    assert_eq!(fx.kind("dst/d"), Some(EntryKind::Symlink), "{label}");
                }
                OverwritePolicy::Skip => {
                    assert_eq!(d, (Action::Skip, ItemStatus::Skipped), "{label}");
                    assert_eq!(report.results[1].skip, Some(SkipReason::Exists));
                    assert_eq!(report.plan.items.len(), 2, "{label}: subtree skipped");
                    assert_eq!(fx.kind("dst/d"), Some(EntryKind::Symlink), "{label}");
                }
                _ => {
                    assert_eq!(d, (Action::CreateDir, ItemStatus::Done), "{label}");
                    assert_eq!(
                        fx.kind("dst/d"),
                        Some(EntryKind::Dir),
                        "{label}: a real folder"
                    );
                    assert_eq!(fx.read("dst/d/new"), b"new", "{label}");
                }
            }
            assert_eq!(
                fx.names("outside"),
                ["keep"],
                "{label}: never written through"
            );
            assert_targets_intact(&fx, &label);
        }
    }
}

// ---- rule 6: copying a link source ------------------------------------------------------------------------

#[test]
fn a3_6_rule6_links_copy_as_links() {
    for fx in fixtures() {
        let label = fx.label();
        fx.file("tree/f", b"f");
        fx.link("tree/lfile", "f", false);
        // A Windows link target takes no `/`: the link would exist and resolve to nothing.
        let up = if cfg!(windows) && !fx.is_mem() {
            r"..\outside"
        } else {
            "../outside"
        };
        fx.link("tree/ldir", up, true);
        fx.link("tree/broken", "missing", false);
        let report = copy(&fx, &[("tree", "copy")], &copy_opts(OverwritePolicy::Fail));
        assert!(report.ok(), "{label}: {:?}", report.results);
        assert_eq!(
            statuses(&report),
            [
                (Action::CreateDir, ItemStatus::Done),
                (Action::Link, ItemStatus::Done),
                (Action::Copy, ItemStatus::Done),
                (Action::Link, ItemStatus::Done),
                (Action::Link, ItemStatus::Done),
            ],
            "{label}"
        );
        for (link, target) in [("broken", "missing"), ("ldir", up), ("lfile", "f")] {
            let entry = fx.lstat(&format!("copy/{link}")).unwrap();
            assert_eq!(entry.kind, EntryKind::Symlink, "{label} {link}");
            assert_eq!(entry.link_target.as_deref(), Some(target), "{label} {link}");
        }
        // Never followed inside a tree: the outside folder is not copied.
        assert_eq!(fx.read("copy/ldir/keep"), b"keep");
        assert_eq!(fx.names("outside"), ["keep"]);
        // LinkPolicy::Skip.
        let opts = CopyOptions {
            links: LinkPolicy::Skip,
            ..copy_opts(OverwritePolicy::Fail)
        };
        let report = copy(&fx, &[("tree", "copy2")], &opts);
        let skipped: Vec<_> = report
            .results
            .iter()
            .filter(|r| r.skip == Some(SkipReason::Symlink))
            .collect();
        assert_eq!(skipped.len(), 3, "{label}");
        assert_eq!(fx.names("copy2"), ["f"], "{label}");
        assert!(report.ok());
    }
}

#[test]
fn a3_6_rule6_target_without_create_symlinks_skips_with_one_warning() {
    let src = Fx::mem(CaseSensitivity::Sensitive);
    let dst = Fx::mem(CaseSensitivity::Sensitive);
    src.file("tree/f", b"f");
    src.link("tree/l1", "f", false);
    src.link("tree/l2", "f", false);
    dst.dir("d");
    dst.memory()
        .set_capabilities(Capabilities::all().difference(Capabilities::CREATE_SYMLINKS));
    let plan = ops::plan_copy(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("tree"), dst.spec("d/tree"))],
        &copy_opts(OverwritePolicy::Fail),
        &Ctl::detached(),
    )
    .unwrap();
    let missing: Vec<_> = plan
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::CapabilityMissing)
        .collect();
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(missing[0].message.contains("create_symlinks"));
    let report = ops::execute(src.vfs(), dst.vfs(), plan, &Ctl::detached());
    let reasons: Vec<Option<SkipReason>> = report.results.iter().map(|r| r.skip).collect();
    assert_eq!(
        reasons,
        [
            None,
            None,
            Some(SkipReason::SymlinksUnsupported),
            Some(SkipReason::SymlinksUnsupported)
        ]
    );
    assert!(report.ok());
    assert_eq!(dst.names("d/tree"), ["f"]);
}

// ---- rule 7: follow_roots ----------------------------------------------------------------------------------

#[test]
fn a3_6_rule7_follow_roots_follows_a_root_only() {
    for fx in fixtures() {
        let label = fx.label();
        fx.link("outside/inner", "../pointed", false);
        // Without follow_roots: the root link is copied as a link.
        let report = copy(
            &fx,
            &[("loutside", "c1"), ("lpointed", "c2")],
            &copy_opts(OverwritePolicy::Fail),
        );
        assert_eq!(
            statuses(&report),
            [
                (Action::Link, ItemStatus::Done),
                (Action::Link, ItemStatus::Done)
            ],
            "{label}"
        );
        assert_eq!(fx.kind("c1"), Some(EntryKind::Symlink));
        assert_eq!(fx.kind("c2"), Some(EntryKind::Symlink));
        // With follow_roots: the root is replaced by what it points to; links inside stay links.
        let opts = CopyOptions {
            follow_roots: true,
            ..copy_opts(OverwritePolicy::Fail)
        };
        let report = copy(&fx, &[("loutside", "f1"), ("lpointed", "f2")], &opts);
        assert!(report.ok(), "{label}: {:?}", report.results);
        assert_eq!(fx.kind("f1"), Some(EntryKind::Dir), "{label}");
        assert_eq!(fx.read("f1/keep"), b"keep");
        assert_eq!(
            fx.kind("f1/inner"),
            Some(EntryKind::Symlink),
            "{label}: inside not followed"
        );
        assert_eq!(fx.kind("f2"), Some(EntryKind::File), "{label}");
        assert_eq!(fx.read("f2"), b"pointed bytes");
        assert_targets_intact(&fx, label);
    }
}

// ---- rule 8: intermediate components are followed ----------------------------------------------------------

#[test]
fn a3_6_rule8_intermediate_links_are_followed() {
    for fx in fixtures() {
        let label = fx.label();
        fx.dir("dst");
        fx.link("ldst", "dst", true);
        // Source and target reached through links to folders.
        let report = copy(
            &fx,
            &[("loutside/keep", "ldst/copied")],
            &copy_opts(OverwritePolicy::Fail),
        );
        assert!(report.ok(), "{label}: {:?}", report.results);
        assert_eq!(fx.read("dst/copied"), b"keep", "{label}");
        fx.vfs().create_dir(&fx.spec("ldst/made"), false).unwrap();
        assert_eq!(fx.kind("dst/made"), Some(EntryKind::Dir), "{label}");
        fx.vfs().remove(&fx.spec("ldst/made")).unwrap();
        assert_eq!(fx.kind("dst/made"), None, "{label}");
        // Only the final component is protected: removing `ldst` removes the link.
        fx.vfs().remove(&fx.spec("ldst")).unwrap();
        assert_eq!(fx.read("dst/copied"), b"keep", "{label}");
    }
}

// ---- rule 9: open_write without replace_link writes through ------------------------------------------------

#[test]
fn a3_6_rule9_open_write_without_replace_link_writes_through() {
    for fx in fixtures() {
        let label = fx.label();
        let save = WriteOptions {
            overwrite: true,
            ..WriteOptions::default()
        };
        let mut w = fx.vfs().open_write(&fx.spec("lpointed"), &save).unwrap();
        w.write_all(b"saved").unwrap();
        w.commit().unwrap();
        assert_eq!(
            fx.kind("lpointed"),
            Some(EntryKind::Symlink),
            "{label}: the link stays"
        );
        assert_eq!(fx.read("pointed"), b"saved", "{label}: written through");
        // With replace_link the link itself is replaced.
        let copy_like = WriteOptions {
            replace_link: true,
            ..save
        };
        let mut w = fx
            .vfs()
            .open_write(&fx.spec("lpointed"), &copy_like)
            .unwrap();
        w.write_all(b"own").unwrap();
        w.commit().unwrap();
        assert_eq!(fx.kind("lpointed"), Some(EntryKind::File), "{label}");
        assert_eq!(fx.read("lpointed"), b"own");
        assert_eq!(fx.read("pointed"), b"saved", "{label}");
    }
}

// ---- W3 review fixes (§1.3.12): links swapped in between planning and execution ------------------------------

/// Replaces the folder `rel` by a link to `outside` (another process, between planning and execution).
fn swap_for_link(fx: &Fx, rel: &str, target: &str) {
    fx.vfs()
        .rename(&fx.spec(rel), &fx.spec("swapped-away"), false)
        .unwrap();
    assert!(fx.link(rel, target, true));
}

#[test]
fn fix_failed_link_copy_keeps_the_target_it_would_replace() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("f", b"orig");
    fx.link("l", "f", false);
    fx.file("dst", b"precious");
    fx.memory().inject(hikasa_vfs::Fault {
        path: fx.text("dst"),
        op: VfsOp::Symlink,
        kind: ErrorKind::PermissionDenied,
    });
    let before = fx.memory().dump();
    let report = copy(&fx, &[("l", "dst")], &copy_opts(OverwritePolicy::Always));
    assert_eq!(statuses(&report), [(Action::Link, ItemStatus::Failed)]);
    assert_eq!(fx.read("dst"), b"precious");
    assert_eq!(
        fx.memory().dump(),
        before,
        "nothing changed, no sibling left"
    );
    // The same through a move into another back-end (the link is copied, then removed).
    let other = Fx::mem(CaseSensitivity::Sensitive);
    other.file("dst", b"precious");
    other.memory().inject(hikasa_vfs::Fault {
        path: other.text("dst"),
        op: VfsOp::Symlink,
        kind: ErrorKind::PermissionDenied,
    });
    let plan = ops::plan_move(
        fx.vfs(),
        other.vfs(),
        &[(fx.spec("l"), other.spec("dst"))],
        &MoveOptions {
            overwrite: OverwritePolicy::Always,
            create_parents: false,
        },
        &Ctl::detached(),
    )
    .unwrap();
    let report = ops::execute(fx.vfs(), other.vfs(), plan, &Ctl::detached());
    assert_eq!(statuses(&report), [(Action::Move, ItemStatus::Failed)]);
    assert_eq!(other.read("dst"), b"precious");
    assert_eq!(fx.kind("l"), Some(EntryKind::Symlink), "the source stays");
    // Without the fault the link replaces the file.
    fx.memory().clear_faults();
    let report = copy(&fx, &[("l", "dst")], &copy_opts(OverwritePolicy::Always));
    assert!(report.ok());
    assert_eq!(fx.kind("dst"), Some(EntryKind::Symlink));
    assert_eq!(
        fx.names("").iter().filter(|n| n.contains("hikasa")).count(),
        0
    );
}

#[test]
fn fix_merged_folder_swapped_for_a_link_is_never_written_through() {
    for fx in fixtures() {
        let label = fx.label();
        fx.file("src/data", b"new");
        fx.dir("dst/src");
        let plan = ops::plan_copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src"), fx.spec("dst/src"))],
            &copy_opts(OverwritePolicy::Fail),
            &Ctl::detached(),
        )
        .unwrap();
        fx.vfs().remove(&fx.spec("dst/src")).unwrap();
        assert!(fx.link("dst/src", "../outside", true));
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert_eq!(
            statuses(&report),
            [
                (Action::KeepDir, ItemStatus::Failed),
                (Action::Copy, ItemStatus::NotRun),
            ],
            "{label}"
        );
        assert!(
            fx.lstat("outside/data").is_none(),
            "{label}: written through the link"
        );
        assert_eq!(fx.names("outside"), ["keep"], "{label}");
    }
}

#[test]
fn fix_remove_never_follows_a_folder_swapped_for_a_link() {
    for fx in fixtures() {
        let label = fx.label();
        fx.file("d/sub/data", b"inside");
        fx.file("d/sub/deep/more", b"inside");
        fx.file("outside/data", b"precious");
        fx.file("outside/deep/more", b"precious");
        let plan = ops::plan_remove(
            fx.vfs(),
            &[fx.spec("d")],
            &RemoveOptions {
                recursive: true,
                trash: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        swap_for_link(&fx, "d/sub", "../outside");
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        let label = format!("{label}: {:#?}", statuses(&report));
        assert_eq!(fx.read("outside/data"), b"precious", "{label}");
        assert_eq!(fx.read("outside/deep/more"), b"precious", "{label}");
        assert_eq!(fx.read("outside/keep"), b"keep", "{label}");
        assert_eq!(fx.kind("d/sub"), Some(EntryKind::Symlink), "{label}");
        // `d/sub`'s contents are not run, `d/sub` itself fails "changed", `d` is kept.
        let sub = report
            .plan
            .items
            .iter()
            .position(|i| i.target == fx.spec("d/sub"))
            .unwrap();
        assert_eq!(report.results[sub].status, ItemStatus::Failed, "{label}");
        assert!(
            report.results[sub]
                .error
                .as_ref()
                .unwrap()
                .message
                .ends_with("changed since the plan was made")
        );
        for (item, result) in report.plan.items.iter().zip(&report.results).take(sub) {
            assert_eq!(result.status, ItemStatus::NotRun, "{label}: {item:?}");
        }
        assert_eq!(
            report.results.last().unwrap().skip,
            Some(SkipReason::ChildrenFailed),
            "{label}"
        );
        assert!(!report.ok());
    }
}

#[test]
fn fix_move_never_removes_through_a_source_folder_swapped_for_a_link() {
    for fx in fixtures() {
        let label = fx.label();
        fx.file("src/t/sub/data", b"inside");
        fx.file("outside/data", b"precious");
        // An existing target folder: the move is expanded (merge), item by item.
        fx.dir("dst/t");
        let plan = ops::plan_move(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src/t"), fx.spec("dst/t"))],
            &MoveOptions::default(),
            &Ctl::detached(),
        )
        .unwrap();
        assert!(
            plan.items.iter().any(|i| i.action == Action::Move),
            "{label}"
        );
        swap_for_link(&fx, "src/t/sub", "../../outside");
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        let shown = format!("{label}: {:#?}", statuses(&report));
        assert_eq!(fx.read("outside/data"), b"precious", "{shown}");
        assert_eq!(fx.read("outside/keep"), b"keep", "{shown}");
        assert!(fx.lstat("dst/t/sub/data").is_none(), "{shown}");
        assert!(!report.ok(), "{shown}");
        let moved = report
            .plan
            .items
            .iter()
            .zip(&report.results)
            .filter(|(i, _)| i.action == Action::Move)
            .all(|(_, r)| r.status == ItemStatus::NotRun);
        assert!(moved, "{shown}");
    }
}
