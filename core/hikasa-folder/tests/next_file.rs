//! M2 W10c acceptance A10.26, its folder part (TC-052, D-288; contract §1.10.21): `next_difference_file`'s candidates
//! and order on a fixture tree (orphans, folders, links and same pairs skipped; Ignored rows; unimportant differences;
//! the display filter; closed folders looked into; `limit_to_folder`; wrap; both directions), a tree of 100,000 rows
//! answered without a list of rows, and P-NEXT-FILE: on random tree pairs, every start, direction and option against a
//! naive scan of the shown rows.

mod common;

use common::{
    compared, config, default_now, engines, gen_rules, gen_tree, host, materialize_gen, run_on,
};
use hikasa_core::{CaseSensitivity, ContentMethod, FileTime, RelPath};
use hikasa_folder::view::{Dir, NextFileOptions, next_difference_file};
use hikasa_folder::{
    Category, DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode, FolderRules,
    FolderToggles, NodeFlags, NodeId, NodeKind, NullObserver, TreeRead,
};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;

// ---- the fixture ---------------------------------------------------------------------------------------------------

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

/// Both sides of one relative path.
fn pair(vfs: &MemoryVfs, rel: &str, left: &[u8], right: &[u8], secs: (i64, i64)) {
    file(vfs, &format!("/l/{rel}"), left, secs.0);
    file(vfs, &format!("/r/{rel}"), right, secs.1);
}

/// The tree. Folders come before files among siblings (D-20), so the pre-order is:
///
/// ```text
/// d/  d/g/  d/g/h.txt (left newer)  d/e.txt (different)  d/f.txt (same)  d/i.txt (different)
/// z/ (left only)  z/q.txt (left only)
/// a.txt (different)  b.txt (same)  c.txt (left only)  j.txt (different, Ignored)  k.txt (right only)
/// m.txt (different)  u.txt (whitespace only: unimportant)
/// ```
fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    pair(&vfs, "d/g/h.txt", b"h", b"H", (20, 10));
    pair(&vfs, "d/e.txt", b"e", b"ee", (10, 10));
    pair(&vfs, "d/f.txt", b"f", b"f", (10, 10));
    pair(&vfs, "d/i.txt", b"i", b"ii", (10, 10));
    file(&vfs, "/l/z/q.txt", b"q", 10);
    pair(&vfs, "a.txt", b"a", b"aa", (10, 10));
    pair(&vfs, "b.txt", b"b", b"b", (10, 10));
    file(&vfs, "/l/c.txt", b"c", 10);
    pair(&vfs, "j.txt", b"j", b"jj", (10, 10));
    file(&vfs, "/r/k.txt", b"k", 10);
    pair(&vfs, "m.txt", b"m", b"mm", (10, 10));
    pair(&vfs, "u.txt", b"u v\n", b"uv \n", (10, 10));
    vfs
}

/// Content by rules (the fake rules comparator reads whitespace as unimportant), `j.txt` marked Ignored.
fn rules() -> FolderRules {
    let mut rules = FolderRules {
        ignored: vec!["j.txt".to_owned()],
        ..FolderRules::default()
    };
    rules.criteria.content.method = ContentMethod::Rules;
    rules
}

fn fixture() -> FolderComparison {
    let cmp = compared(&tree(), &rules());
    cmp.check_invariants().unwrap();
    cmp
}

fn id(read: &TreeRead<'_>, path: &str) -> NodeId {
    read.node_at(&RelPath::parse(path).unwrap())
        .unwrap_or_else(|| panic!("no row {path}"))
}

fn path(read: &TreeRead<'_>, node: Option<NodeId>) -> Option<String> {
    node.map(|n| read.path(n).as_str().to_owned())
}

fn everything() -> DisplayOptions {
    DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    }
}

fn opts() -> NextFileOptions {
    NextFileOptions {
        limit_to_folder: false,
        wrap: false,
        display: everything(),
        ignore_unimportant: false,
    }
}

/// Every answer from `from` on, following `dir` until `None` (or a repeat, with wrap).
fn chain(read: &TreeRead<'_>, from: &str, dir: Dir, opts: &NextFileOptions) -> Vec<String> {
    let mut at = if from.is_empty() {
        NodeId::ROOT
    } else {
        id(read, from)
    };
    let mut out = Vec::new();
    while let Some(next) = next_difference_file(read, at, dir, opts) {
        let p = read.path(next).as_str().to_owned();
        if out.contains(&p) {
            break;
        }
        out.push(p);
        at = next;
    }
    out
}

// ---- the fixture's categories, so the tests below mean what they say --------------------------------------------------

#[test]
fn a10_26_the_fixture_is_what_the_tests_say() {
    let cmp = fixture();
    let read = cmp.read();
    let order: Vec<String> = read
        .preorder()
        .map(|(n, _)| read.path(n).as_str().to_owned())
        .collect();
    assert_eq!(
        order,
        [
            "d",
            "d/g",
            "d/g/h.txt",
            "d/e.txt",
            "d/f.txt",
            "d/i.txt",
            "z",
            "z/q.txt",
            "a.txt",
            "b.txt",
            "c.txt",
            "j.txt",
            "k.txt",
            "m.txt",
            "u.txt"
        ]
    );
    let cat = |p: &str| read.category(id(&read, p), false);
    assert_eq!(cat("d/g/h.txt"), Category::LeftNewer);
    assert_eq!(cat("d/e.txt"), Category::Different);
    assert_eq!(cat("d/f.txt"), Category::Same);
    assert_eq!(cat("z/q.txt"), Category::LeftOrphan);
    assert_eq!(cat("c.txt"), Category::LeftOrphan);
    assert_eq!(cat("k.txt"), Category::RightOrphan);
    assert_eq!(cat("j.txt"), Category::Same, "Ignored reads as same");
    assert!(
        read.node(id(&read, "j.txt"))
            .unwrap()
            .flags
            .contains(NodeFlags::IGNORED)
    );
    assert_eq!(cat("u.txt"), Category::Different);
    assert_eq!(
        read.category(id(&read, "u.txt"), true),
        Category::Same,
        "unimportant only"
    );
    assert_eq!(read.node(id(&read, "d")).unwrap().kind, NodeKind::Folder);
}

// ---- candidates and order ------------------------------------------------------------------------------------------------

#[test]
fn a10_26_candidates_in_pre_order_both_directions() {
    let cmp = fixture();
    let read = cmp.read();
    let all = ["d/g/h.txt", "d/e.txt", "d/i.txt", "a.txt", "m.txt", "u.txt"];
    // From the root: the first candidate and on; orphans (z/q.txt, c.txt, k.txt), folders, same pairs (d/f.txt,
    // b.txt) and the Ignored j.txt are never targets.
    assert_eq!(chain(&read, "", Dir::Forward, &opts()), all);
    let mut backward: Vec<&str> = all.to_vec();
    backward.reverse();
    assert_eq!(chain(&read, "", Dir::Backward, &opts()), backward);
    // From a row in the middle, and from rows that are not candidates themselves.
    assert_eq!(
        chain(&read, "d/i.txt", Dir::Forward, &opts()),
        ["a.txt", "m.txt", "u.txt"]
    );
    assert_eq!(
        chain(&read, "d/f.txt", Dir::Backward, &opts()),
        ["d/e.txt", "d/g/h.txt"]
    );
    assert_eq!(
        chain(&read, "z/q.txt", Dir::Forward, &opts()),
        ["a.txt", "m.txt", "u.txt"]
    );
    assert_eq!(
        chain(&read, "j.txt", Dir::Backward, &opts()),
        ["a.txt", "d/i.txt", "d/e.txt", "d/g/h.txt"]
    );
    // From a folder: forward goes into it, backward before it.
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "d"), Dir::Forward, &opts())
        ),
        Some("d/g/h.txt".into())
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "z"), Dir::Backward, &opts())
        ),
        Some("d/i.txt".into())
    );
    // The last one has no next, the first no previous (no wrap): the view's message.
    assert_eq!(
        next_difference_file(&read, id(&read, "u.txt"), Dir::Forward, &opts()),
        None
    );
    assert_eq!(
        next_difference_file(&read, id(&read, "d/g/h.txt"), Dir::Backward, &opts()),
        None
    );
    // An id this model never issued.
    assert_eq!(
        next_difference_file(&read, NodeId(1_000_000), Dir::Forward, &opts()),
        None
    );
    // The method on the guard is the same function.
    assert_eq!(
        read.next_difference_file(NodeId::ROOT, Dir::Forward, &opts()),
        Some(id(&read, "d/g/h.txt"))
    );
}

#[test]
fn a10_26_unimportant_differences_are_skipped_with_ignore_unimportant() {
    let cmp = fixture();
    let read = cmp.read();
    let iu = NextFileOptions {
        ignore_unimportant: true,
        ..opts()
    };
    assert_eq!(
        chain(&read, "a.txt", Dir::Forward, &iu),
        ["m.txt"],
        "u.txt has only unimportant differences"
    );
    assert_eq!(
        chain(&read, "a.txt", Dir::Forward, &opts()),
        ["m.txt", "u.txt"]
    );
}

#[test]
fn a10_26_the_display_filter_decides_what_is_shown() {
    let cmp = fixture();
    let read = cmp.read();
    // Newer only: the left-newer pair alone.
    let newer = NextFileOptions {
        display: DisplayOptions {
            filter: FolderDisplayFilter::Toggles {
                bits: (FolderToggles::LEFT_NEWER | FolderToggles::RIGHT_NEWER).bits(),
            },
            ..everything()
        },
        ..opts()
    };
    assert_eq!(chain(&read, "", Dir::Forward, &newer), ["d/g/h.txt"]);
    assert_eq!(chain(&read, "a.txt", Dir::Backward, &newer), ["d/g/h.txt"]);
    // Differences without the newer ones: the different pairs.
    let different = NextFileOptions {
        display: DisplayOptions {
            filter: FolderDisplayFilter::Toggles {
                bits: FolderToggles::DIFFERENCES.bits(),
            },
            ..everything()
        },
        ..opts()
    };
    assert_eq!(
        chain(&read, "", Dir::Forward, &different),
        ["d/e.txt", "d/i.txt", "a.txt", "m.txt", "u.txt"]
    );
    // Same only, or orphans only: nothing to go to.
    for filter in [FolderDisplayFilter::Same, FolderDisplayFilter::Orphans] {
        let hidden = NextFileOptions {
            display: DisplayOptions {
                filter,
                ..everything()
            },
            wrap: true,
            ..opts()
        };
        assert_eq!(
            chain(&read, "", Dir::Forward, &hidden),
            Vec::<String>::new()
        );
    }
    // The display's own ignore-unimportant hides u.txt from the view even when the search does not skip it.
    let view_iu = NextFileOptions {
        display: DisplayOptions {
            filter: FolderDisplayFilter::Differences,
            ignore_unimportant: true,
            ..everything()
        },
        ..opts()
    };
    assert_eq!(chain(&read, "a.txt", Dir::Forward, &view_iu), ["m.txt"]);
    // Every folder mode shows the folders of a shown file.
    for folders in FolderMode::ALL {
        let mode = NextFileOptions {
            display: DisplayOptions {
                filter: FolderDisplayFilter::Differences,
                folders,
                ..everything()
            },
            ..opts()
        };
        assert_eq!(
            chain(&read, "", Dir::Forward, &mode),
            ["d/g/h.txt", "d/e.txt", "d/i.txt", "a.txt", "m.txt", "u.txt"],
            "{folders:?}"
        );
    }
}

#[test]
fn a10_26_from_a_row_the_display_hides() {
    let cmp = fixture();
    let read = cmp.read();
    // Differences only: the same pair d/f.txt and the folder z are hidden; the search still starts at their place.
    let differences = NextFileOptions {
        display: DisplayOptions {
            filter: FolderDisplayFilter::DifferencesNoOrphans,
            ..everything()
        },
        ..opts()
    };
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "d/f.txt"), Dir::Forward, &differences)
        ),
        Some("d/i.txt".into())
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "z/q.txt"), Dir::Backward, &differences)
        ),
        Some("d/i.txt".into())
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "z/q.txt"), Dir::Forward, &differences)
        ),
        Some("a.txt".into())
    );
}

#[test]
fn a10_26_closed_folders_are_looked_into() {
    // The function has no notion of an open folder: every scanned row counts (D-241's rule), so a pair two folders
    // down is found from the top whatever the view shows open.
    let cmp = fixture();
    let read = cmp.read();
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, NodeId::ROOT, Dir::Forward, &opts())
        ),
        Some("d/g/h.txt".into())
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "d/e.txt"), Dir::Backward, &opts())
        ),
        Some("d/g/h.txt".into())
    );
}

#[test]
fn a10_26_limit_to_folder() {
    let cmp = fixture();
    let read = cmp.read();
    let limit = NextFileOptions {
        limit_to_folder: true,
        ..opts()
    };
    // In d: its files only, not d/g's.
    assert_eq!(chain(&read, "d/e.txt", Dir::Forward, &limit), ["d/i.txt"]);
    assert_eq!(chain(&read, "d/i.txt", Dir::Backward, &limit), ["d/e.txt"]);
    assert_eq!(
        next_difference_file(&read, id(&read, "d/e.txt"), Dir::Backward, &limit),
        None,
        "d/g/h.txt is below a sub-folder"
    );
    // At the top: the base's files.
    assert_eq!(
        chain(&read, "b.txt", Dir::Forward, &limit),
        ["m.txt", "u.txt"]
    );
    assert_eq!(
        chain(&read, "", Dir::Forward, &limit),
        ["a.txt", "m.txt", "u.txt"]
    );
    // With wrap, inside the folder.
    let wrap = NextFileOptions {
        wrap: true,
        ..limit
    };
    assert_eq!(
        chain(&read, "d/i.txt", Dir::Forward, &wrap),
        ["d/e.txt", "d/i.txt"]
    );
    assert_eq!(
        chain(&read, "a.txt", Dir::Backward, &wrap),
        ["u.txt", "m.txt", "a.txt"]
    );
    // A folder with one candidate: from it, nothing else (it is never its own next).
    assert_eq!(
        next_difference_file(&read, id(&read, "d/g/h.txt"), Dir::Forward, &wrap),
        None
    );
}

#[test]
fn a10_26_wrap() {
    let cmp = fixture();
    let read = cmp.read();
    let wrap = NextFileOptions {
        wrap: true,
        ..opts()
    };
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "u.txt"), Dir::Forward, &wrap)
        ),
        Some("d/g/h.txt".into())
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "d/g/h.txt"), Dir::Backward, &wrap)
        ),
        Some("u.txt".into())
    );
    // Around once, back to the start.
    assert_eq!(
        chain(&read, "a.txt", Dir::Forward, &wrap),
        ["m.txt", "u.txt", "d/g/h.txt", "d/e.txt", "d/i.txt", "a.txt"]
    );
    assert_eq!(
        chain(&read, "a.txt", Dir::Backward, &wrap),
        ["d/i.txt", "d/e.txt", "d/g/h.txt", "u.txt", "m.txt", "a.txt"]
    );
    // From a folder going backward, its own rows come last.
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "z"), Dir::Backward, &wrap)
        ),
        Some("d/i.txt".into())
    );
    // A single candidate is never its own next.
    let newer = NextFileOptions {
        display: DisplayOptions {
            filter: FolderDisplayFilter::LeftNewer,
            ..everything()
        },
        ..wrap
    };
    assert_eq!(
        next_difference_file(&read, id(&read, "d/g/h.txt"), Dir::Forward, &newer),
        None
    );
    assert_eq!(
        path(
            &read,
            next_difference_file(&read, id(&read, "a.txt"), Dir::Forward, &newer)
        ),
        Some("d/g/h.txt".into())
    );
}

// ---- a large tree --------------------------------------------------------------------------------------------------------

#[test]
fn a10_26_a_tree_of_100000_rows_answers_in_milliseconds() {
    // 100 folders of 1,000 files, every file the same but the last one of the last folder.
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for f in 0..100 {
        for n in 0..1000 {
            let rel = format!("f{f:03}/n{n:04}.txt");
            let right: &[u8] = if f == 99 && n == 999 { b"yy" } else { b"x" };
            pair(&vfs, &rel, b"x", right, (10, 10));
        }
    }
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    assert!(read.len() > 100_000);
    let wrap = NextFileOptions {
        wrap: true,
        ..opts()
    };
    let first = id(&read, "f000/n0000.txt");
    let started = std::time::Instant::now();
    let mut found = None;
    for _ in 0..10 {
        found = next_difference_file(&read, first, Dir::Forward, &wrap);
        assert_eq!(
            next_difference_file(&read, first, Dir::Backward, &wrap),
            found
        );
    }
    let elapsed = started.elapsed();
    assert_eq!(path(&read, found), Some("f099/n0999.txt".into()));
    // 20 walks over the whole tree; a debug build on a loaded machine stays far below this.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "20 walks of 100,000 rows took {elapsed:?}"
    );
}

// ---- P-NEXT-FILE ------------------------------------------------------------------------------------------------------

/// The naive answer: the shown rows (every folder open) in pre-order, the candidates among them by the public API, the
/// next one after `from`'s place in the full pre-order.
fn naive(read: &TreeRead<'_>, from: NodeId, dir: Dir, opts: &NextFileOptions) -> Option<NodeId> {
    let order: Vec<NodeId> = std::iter::once(NodeId::ROOT)
        .chain(read.preorder().map(|(n, _)| n))
        .collect();
    let position = |n: NodeId| order.iter().position(|&o| o == n);
    let mut shown = Vec::new();
    read.visible_rows(&opts.display, &|_| true, &mut shown);
    let parent = read.node(from)?.parent;
    let candidates: Vec<NodeId> = shown
        .iter()
        .map(|row| row.node)
        .filter(|&n| {
            let view = read.node(n).unwrap();
            view.kind == NodeKind::File
                && view.left.is_some()
                && view.right.is_some()
                && !view.flags.contains(NodeFlags::IGNORED)
                && matches!(
                    read.category(n, opts.ignore_unimportant),
                    Category::Different | Category::LeftNewer | Category::RightNewer
                )
        })
        .filter(|&n| {
            !opts.limit_to_folder
                || read.node(n).unwrap().parent
                    == if from == NodeId::ROOT {
                        Some(NodeId::ROOT)
                    } else {
                        parent
                    }
        })
        .filter(|&n| n != from)
        .collect();
    let at = position(from)?;
    // The root stands before the first row going forward, after the last one going backward.
    let (after, before): (Vec<NodeId>, Vec<NodeId>) = candidates.iter().partition(|&&n| {
        from == NodeId::ROOT && dir == Dir::Forward
            || from != NodeId::ROOT && position(n).unwrap() > at
    });
    match dir {
        Dir::Forward => after.first().copied().or_else(|| {
            if opts.wrap {
                before.first().copied()
            } else {
                None
            }
        }),
        Dir::Backward => before.last().copied().or_else(|| {
            if opts.wrap {
                after.last().copied()
            } else {
                None
            }
        }),
    }
}

fn display_options() -> impl Strategy<Value = DisplayOptions> {
    (
        prop_oneof![
            Just(FolderDisplayFilter::All),
            Just(FolderDisplayFilter::Differences),
            Just(FolderDisplayFilter::DifferencesNoOrphans),
            Just(FolderDisplayFilter::LeftNewer),
            Just(FolderDisplayFilter::Same),
            (0u8..64).prop_map(|bits| FolderDisplayFilter::Toggles { bits }),
        ],
        prop_oneof![
            Just(FolderMode::AlwaysShowFolders),
            Just(FolderMode::CompareStructure),
            Just(FolderMode::OnlyFiles),
        ],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(filter, folders, ignore_unimportant, suppress_filters)| DisplayOptions {
                filter,
                folders,
                ignore_unimportant,
                suppress_filters,
            },
        )
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/next_file.txt"))]

    /// P-NEXT-FILE: from every row and the root, both directions, with and without wrap and the folder limit,
    /// `next_difference_file` equals the naive scan.
    #[test]
    fn p_next_file_equals_a_naive_scan(
        nodes in gen_tree(),
        gr in gen_rules(),
        display in display_options(),
        ignore_unimportant in any::<bool>(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        let engines = engines(64);
        let cmp = run_on(
            &vfs,
            &gr.rules(true),
            None,
            &engines,
            &default_now(),
            &NullObserver,
            &hikasa_core::CancelToken::new(),
            false,
        )
        .unwrap();
        let read = cmp.read();
        let rows: Vec<NodeId> = std::iter::once(NodeId::ROOT)
            .chain(read.preorder().map(|(n, _)| n))
            .collect();
        for &from in &rows {
            for dir in [Dir::Forward, Dir::Backward] {
                for limit_to_folder in [false, true] {
                    for wrap in [false, true] {
                        let opts = NextFileOptions {
                            limit_to_folder,
                            wrap,
                            display,
                            ignore_unimportant,
                        };
                        prop_assert_eq!(
                            next_difference_file(&read, from, dir, &opts),
                            naive(&read, from, dir, &opts),
                            "from {:?} ({}) {:?} {:?}",
                            from,
                            read.path(from).as_str(),
                            dir,
                            opts
                        );
                    }
                }
            }
        }
    }
}
