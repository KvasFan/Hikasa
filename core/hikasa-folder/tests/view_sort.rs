//! M2 W9a acceptance A9.2 (D-235): `TreeRead::visible_rows_sorted`: every `SortColumn` both ways on both sides, folders
//! first, a row missing on the sorted side, ties by the D-20 key, a flat comparison, `Name` ascending equal to
//! `visible_rows`; property: for random trees and keys the sorted rows are a permutation of `visible_rows` that keeps
//! every row under its parent and orders each folder's children by the key.

mod common;

use std::collections::BTreeMap;

use common::{compared, config, gen_rules, gen_tree, host, materialize_gen};
use hikasa_core::{CaseSensitivity, FileTime, Side};
use hikasa_folder::view::{SortColumn, SortKey, attributes_text, compare_rows, extension};
use hikasa_folder::{
    Attrs, DisplayOptions, DosAttrs, FolderComparison, FolderRules, FolderRulesPatch, NodeId,
    TreeRead, UnixMeta, VisibleRow,
};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;

fn meta(secs: Option<i64>, attrs: Option<Attrs>, crc: Option<u32>) -> MemMeta {
    MemMeta {
        mtime: secs.and_then(|s| FileTime::from_unix(s, 0)),
        attrs,
        crc32: crc,
        ..MemMeta::default()
    }
}

fn unix(mode: u32, uid: u32, gid: u32) -> Option<Attrs> {
    Some(Attrs::Unix {
        meta: UnixMeta { mode, uid, gid },
    })
}

/// The fixture: a folder `zeta` (sorted first as a folder whatever the key), files with distinct sizes, times,
/// extensions, modes, owners, groups and CRCs on the left; on the right `b.txt` alone differs, `only-left.md` is
/// missing and `only-right.c` exists.
fn fixture() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
    let file = |path: &str, bytes: usize, m: MemMeta| {
        vfs.insert_file(&host(path), vec![b'x'; bytes], m).unwrap();
    };
    for side in ["l", "r"] {
        vfs.insert_dir(&host(&format!("/{side}/zeta")), MemMeta::default())
            .unwrap();
        file(
            &format!("/{side}/zeta/inner"),
            50,
            meta(Some(10), None, None),
        );
        file(
            &format!("/{side}/a.rs"),
            30,
            meta(Some(300), unix(0o644, 7, 3), Some(9)),
        );
        file(
            &format!("/{side}/C.py"),
            10,
            meta(Some(100), unix(0o600, 5, 9), Some(2)),
        );
    }
    file("/l/b.txt", 20, meta(Some(200), unix(0o755, 6, 1), Some(5)));
    file("/r/b.txt", 40, meta(Some(400), unix(0o755, 1, 8), Some(1)));
    file("/l/only-left.md", 5, meta(None, unix(0o400, 9, 2), None));
    file(
        "/r/only-right.c",
        25,
        meta(Some(250), unix(0o640, 2, 2), Some(7)),
    );
    vfs
}

fn crc_rules() -> FolderRules {
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            unix: Some(hikasa_folder::UnixCriteria {
                permissions: false,
                owner: false,
                group: false,
            }),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    rules
}

fn all() -> DisplayOptions {
    DisplayOptions {
        filter: hikasa_folder::FolderDisplayFilter::All,
        folders: hikasa_folder::FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    }
}

/// The names (left, else right) of the sorted rows, with `/` before the children of an open folder.
fn names(cmp: &FolderComparison, sort: Option<&SortKey>) -> Vec<String> {
    let read = cmp.read();
    let mut rows = Vec::new();
    read.visible_rows_sorted(&all(), &|_| true, sort, &mut rows);
    rows.iter()
        .map(|r| {
            let node = read.node(r.node).unwrap();
            let name = node.representative().unwrap().name.to_owned();
            format!("{}{name}", "  ".repeat(r.depth as usize))
        })
        .collect()
}

fn key(column: SortColumn, side: Side, descending: bool) -> SortKey {
    SortKey {
        column,
        side,
        descending,
    }
}

#[test]
fn every_column_both_ways_on_both_sides() {
    let cmp = compared(&fixture(), &crc_rules());
    // The tree order (D-20: folders first, then by the case-folded name).
    let tree = names(&cmp, None);
    assert_eq!(
        tree,
        [
            "zeta",
            "  inner",
            "a.rs",
            "b.txt",
            "C.py",
            "only-left.md",
            "only-right.c"
        ]
    );
    // Files only: the folder stays first in every order; `inner` alone below it.
    let files = |sort: SortKey| -> Vec<String> {
        let all = names(&cmp, Some(&sort));
        assert_eq!(all[0], "zeta", "{sort:?}");
        assert_eq!(all[1], "  inner", "{sort:?}");
        all[2..].to_vec()
    };
    let cases: &[(SortColumn, Side, [&str; 5])] = &[
        // Name: case-folded.
        (
            SortColumn::Name,
            Side::Left,
            ["a.rs", "b.txt", "C.py", "only-left.md", "only-right.c"],
        ),
        // Ext: c, md, py, rs, txt.
        (
            SortColumn::Ext,
            Side::Left,
            ["only-right.c", "only-left.md", "C.py", "a.rs", "b.txt"],
        ),
        // Size left: 5, 10, 20, 25 (right's, missing on the left), 30.
        (
            SortColumn::Size,
            Side::Left,
            ["only-left.md", "C.py", "b.txt", "only-right.c", "a.rs"],
        ),
        // Size right: 5 (left's), 10, 25, 30, 40.
        (
            SortColumn::Size,
            Side::Right,
            ["only-left.md", "C.py", "only-right.c", "a.rs", "b.txt"],
        ),
        // Modified left: none first, 100, 200, 250, 300.
        (
            SortColumn::Modified,
            Side::Left,
            ["only-left.md", "C.py", "b.txt", "only-right.c", "a.rs"],
        ),
        // Modified right: none, 100, 250, 300, 400.
        (
            SortColumn::Modified,
            Side::Right,
            ["only-left.md", "C.py", "only-right.c", "a.rs", "b.txt"],
        ),
        // Attributes (mode text): -r--------, -rw-------, -rw-r-----, -rw-r--r--, -rwxr-xr-x.
        (
            SortColumn::Attributes,
            Side::Left,
            ["only-left.md", "C.py", "only-right.c", "a.rs", "b.txt"],
        ),
        // CRC left: unknown first, 2, 5, 7, 9.
        (
            SortColumn::Crc,
            Side::Left,
            ["only-left.md", "C.py", "b.txt", "only-right.c", "a.rs"],
        ),
        // CRC right: unknown, 1, 2, 7, 9.
        (
            SortColumn::Crc,
            Side::Right,
            ["only-left.md", "b.txt", "C.py", "only-right.c", "a.rs"],
        ),
        // Owner left: 2, 5, 6, 7, 9.
        (
            SortColumn::Owner,
            Side::Left,
            ["only-right.c", "C.py", "b.txt", "a.rs", "only-left.md"],
        ),
        // Owner right: 1, 2, 5, 7, 9.
        (
            SortColumn::Owner,
            Side::Right,
            ["b.txt", "only-right.c", "C.py", "a.rs", "only-left.md"],
        ),
        // Group left: 1, 2, 2 (tie: tree order), 3, 9.
        (
            SortColumn::Group,
            Side::Left,
            ["b.txt", "only-left.md", "only-right.c", "a.rs", "C.py"],
        ),
        // Group right: 2, 2 (tie), 3, 8, 9.
        (
            SortColumn::Group,
            Side::Right,
            ["only-left.md", "only-right.c", "a.rs", "b.txt", "C.py"],
        ),
    ];
    for (column, side, expected) in cases {
        let up = files(key(*column, *side, false));
        assert_eq!(up, expected, "{column:?} {side:?} ascending");
        // Descending reverses the key; ties keep the tree order.
        let down = files(key(*column, *side, true));
        let mut reversed: Vec<&str> = expected.to_vec();
        reversed.reverse();
        if *column == SortColumn::Group {
            // The tied pair keeps its tree order both ways.
            let at = reversed.iter().position(|n| *n == "only-right.c").unwrap();
            reversed.swap(at, at + 1);
        }
        assert_eq!(down, reversed, "{column:?} {side:?} descending");
    }
    // Ext and Name of the left: both sides agree for the names.
    assert_eq!(
        files(key(SortColumn::Name, Side::Right, false)),
        files(key(SortColumn::Name, Side::Left, false))
    );
    // A folder's size is its total: zeta (50) before nothing else, as a folder; its child alone.
    assert_eq!(
        names(&cmp, Some(&key(SortColumn::Size, Side::Left, true)))[0],
        "zeta"
    );
}

#[test]
fn folders_first_and_sorted_among_themselves() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for side in ["l", "r"] {
        for (name, bytes) in [("big", 90usize), ("small", 1)] {
            vfs.insert_dir(&host(&format!("/{side}/{name}")), MemMeta::default())
                .unwrap();
            vfs.insert_file(
                &host(&format!("/{side}/{name}/f")),
                vec![0; bytes],
                MemMeta::default(),
            )
            .unwrap();
        }
        vfs.insert_file(
            &host(&format!("/{side}/huge")),
            vec![0; 500],
            MemMeta::default(),
        )
        .unwrap();
    }
    let cmp = compared(&vfs, &FolderRules::default());
    let up = names(&cmp, Some(&key(SortColumn::Size, Side::Left, false)));
    assert_eq!(up, ["small", "  f", "big", "  f", "huge"]);
    let down = names(&cmp, Some(&key(SortColumn::Size, Side::Left, true)));
    assert_eq!(down, ["big", "  f", "small", "  f", "huge"]);
}

#[test]
fn a_flat_comparison_sorts_its_one_list_and_the_path_column() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for side in ["l", "r"] {
        for dir in ["", "/a", "/b", "/c", "/c/a"] {
            vfs.insert_dir(&host(&format!("/{side}{dir}")), MemMeta::default())
                .unwrap();
        }
        for path in ["b/x", "a/y", "c/a/z", "w"] {
            vfs.insert_file(
                &host(&format!("/{side}/{path}")),
                b"1".to_vec(),
                MemMeta::default(),
            )
            .unwrap();
        }
    }
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            flatten: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let cmp = compared(&vfs, &rules);
    let by_path = names(&cmp, Some(&key(SortColumn::Path, Side::Left, false)));
    assert_eq!(by_path, ["w", "y", "x", "z"]);
    let by_name = names(&cmp, Some(&key(SortColumn::Name, Side::Right, true)));
    assert_eq!(by_name, ["z", "y", "x", "w"]);
}

#[test]
fn name_ascending_is_the_tree_order_and_texts() {
    let cmp = compared(&fixture(), &crc_rules());
    assert_eq!(
        names(&cmp, None),
        names(&cmp, Some(&key(SortColumn::Name, Side::Left, false)))
    );
    assert_eq!(extension("a.tar.gz"), Some("gz"));
    assert_eq!(extension(".bashrc"), None);
    assert_eq!(extension("a."), None);
    assert_eq!(extension("plain"), None);
    let read = cmp.read();
    let node = (0..read.len())
        .map(NodeId)
        .find(|&id| {
            read.node(id)
                .and_then(|n| n.left)
                .is_some_and(|e| e.name == "b.txt")
        })
        .unwrap();
    let view = read.node(node).unwrap();
    assert_eq!(attributes_text(view.left.as_ref().unwrap()), "-rwxr-xr-x");
    let dos = hikasa_folder::EntryView {
        attrs: Attrs::Dos {
            attrs: DosAttrs::READONLY | DosAttrs::HIDDEN,
        },
        ..*view.left.as_ref().unwrap()
    };
    assert_eq!(attributes_text(&dos), "HR");
    for column in SortColumn::ALL {
        assert_eq!(SortColumn::parse(column.as_str()), Some(column));
    }
}

/// The rows of `read` under `sort`, checked against `visible_rows`: the same multiset, every row's parent the nearest
/// shallower row above it, and each folder's listed children ordered by the key (ties in tree order).
fn check_sorted(
    read: &TreeRead<'_>,
    opts: &DisplayOptions,
    sort: &SortKey,
) -> Result<(), TestCaseError> {
    let open = |_: NodeId| true;
    let mut plain = Vec::new();
    read.visible_rows(opts, &open, &mut plain);
    let mut sorted = Vec::new();
    read.visible_rows_sorted(opts, &open, Some(sort), &mut sorted);
    let mut a: Vec<VisibleRow> = plain.clone();
    let mut b: Vec<VisibleRow> = sorted.clone();
    a.sort_by_key(|r| r.node);
    b.sort_by_key(|r| r.node);
    prop_assert_eq!(a, b);
    // Parents: the nearest row above with depth - 1 is the parent.
    let mut stack: Vec<NodeId> = Vec::new();
    let mut siblings: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for row in &sorted {
        stack.truncate(row.depth as usize);
        let parent = stack.last().copied().unwrap_or(NodeId::ROOT);
        prop_assert_eq!(read.node(row.node).unwrap().parent, Some(parent));
        siblings.entry(parent).or_default().push(row.node);
        stack.push(row.node);
    }
    // Order of siblings by the key, ties by the tree position.
    for (parent, kids) in &siblings {
        let position = |id: NodeId| read.children(*parent).position(|c| c == id).unwrap();
        for pair in kids.windows(2) {
            let order = compare_rows(read, pair[0], pair[1], sort)
                .then(position(pair[0]).cmp(&position(pair[1])));
            prop_assert_eq!(order, std::cmp::Ordering::Less, "{:?} {:?}", pair, sort);
        }
    }
    if sort.column == SortColumn::Name && !sort.descending && sort.side == Side::Left {
        prop_assert_eq!(plain, sorted);
    }
    Ok(())
}

fn gen_key() -> impl Strategy<Value = SortKey> {
    (0..SortColumn::ALL.len(), any::<bool>(), any::<bool>()).prop_map(|(c, right, descending)| {
        SortKey {
            column: SortColumn::ALL[c],
            side: if right { Side::Right } else { Side::Left },
            descending,
        }
    })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/view_sort.txt"))]

    /// A9.2: sorted rows are a permutation of `visible_rows` that keeps every row under its parent and is ordered by
    /// the key; `Name` ascending on the left is the tree order.
    #[test]
    fn p_view_sort(
        nodes in gen_tree(),
        gr in gen_rules(),
        sort in gen_key(),
        filter in 0..hikasa_folder::FolderDisplayFilter::PRESETS.len(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        let cmp = compared(&vfs, &gr.rules(false));
        let opts = DisplayOptions {
            filter: hikasa_folder::FolderDisplayFilter::PRESETS[filter],
            ..all()
        };
        check_sorted(&cmp.read(), &opts, &sort)?;
        check_sorted(&cmp.read(), &all(), &sort)?;
    }
}
