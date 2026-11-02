//! M2 W6b acceptance A6.14 (D-175, D-176): the selection engine, on a fixture holding every category, `resolve`, `open_pair`,
//! `Finder`, and the word table of §1.6.9.

mod common;

use common::{compared, host};
use hikasa_core::filter::NameFilterLists;
use hikasa_core::{CaseSensitivity, FileTime, RelPath, Side};
use hikasa_folder::select::{
    FindQuery, Finder, Include, SelKind, SelSide, SelState, SelectMask, SelectScope, Selection,
    SideSet, open_pair, resolve, select, select_all_files,
};
use hikasa_folder::{
    Criteria, DisplayOptions, FilterRules, FolderComparison, FolderDisplayFilter, FolderError,
    FolderMode, FolderRules, NodeId, ScanStatus, TreeRead,
};
use hikasa_vfs::{MemMeta, MemoryVfs};

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

/// `/l` and `/r`, empty.
fn bases() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    vfs
}

/// Every category × file, folder, empty folder, link.
fn tree() -> MemoryVfs {
    let vfs = bases();
    file(&vfs, "/l/same", b"s", 10);
    file(&vfs, "/r/same", b"s", 10);
    file(&vfs, "/l/newer", b"nn", 20);
    file(&vfs, "/r/newer", b"n", 10);
    file(&vfs, "/l/older", b"o", 10);
    file(&vfs, "/r/older", b"oo", 20);
    file(&vfs, "/l/timeonly", b"t", 20);
    file(&vfs, "/r/timeonly", b"t", 10);
    file(&vfs, "/l/diffsame", b"a", 10);
    file(&vfs, "/r/diffsame", b"bb", 10);
    file(&vfs, "/l/lonly", b"x", 10);
    file(&vfs, "/r/ronly", b"x", 10);
    file(&vfs, "/l/alls/a", b"x", 10);
    file(&vfs, "/r/alls/a", b"x", 10);
    file(&vfs, "/l/mixed/same", b"x", 10);
    file(&vfs, "/r/mixed/same", b"x", 10);
    file(&vfs, "/l/mixed/other", b"yy", 20);
    file(&vfs, "/r/mixed/other", b"y", 10);
    file(&vfs, "/l/lfolder/f", b"x", 10);
    vfs.insert_dir(&host("/l/emptyboth"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r/emptyboth"), meta(1)).unwrap();
    vfs.insert_dir(&host("/l/emptyleft"), meta(1)).unwrap();
    vfs.insert_dir(&host("/l/hollow/sub"), meta(1)).unwrap();
    vfs.insert_symlink(&host("/l/link"), "same", meta(10))
        .unwrap();
    vfs.insert_symlink(&host("/r/link"), "same", meta(10))
        .unwrap();
    vfs
}

fn display(filter: FolderDisplayFilter) -> DisplayOptions {
    DisplayOptions {
        filter,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    }
}

fn all() -> DisplayOptions {
    display(FolderDisplayFilter::All)
}

fn id(read: &TreeRead<'_>, path: &str) -> NodeId {
    read.node_at(&RelPath::parse(path).unwrap()).unwrap()
}

/// What `word` selects, as `<path>:<l|r|lr>`, sorted.
fn picked(cmp: &FolderComparison, word: &str) -> Vec<String> {
    let read = cmp.read();
    let shown = all();
    let scope = SelectScope {
        display: &shown,
        expanded: None,
    };
    let mut sel = Selection::default();
    select(
        &read,
        &scope,
        &[SelectMask::parse(word).unwrap()],
        false,
        &mut sel,
    );
    words(&read, &sel)
}

fn words(read: &TreeRead<'_>, sel: &Selection) -> Vec<String> {
    let mut out: Vec<String> = sel
        .iter()
        .map(|(node, sides)| {
            let side = match (
                sides.contains(SideSet::LEFT),
                sides.contains(SideSet::RIGHT),
            ) {
                (true, true) => "lr",
                (true, false) => "l",
                _ => "r",
            };
            format!("{}:{side}", read.path(node).as_str())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a6_14_the_words_of_the_table() {
    let parse = |word: &str| SelectMask::parse(word).unwrap();
    let result = |side, state, kind| SelectMask::Result { side, state, kind };
    use SelKind as K;
    use SelSide as S;
    use SelState as T;
    for word in ["all", "all.all.all", "ALL", "All.All"] {
        assert_eq!(parse(word), result(S::All, T::All, K::All), "{word}");
    }
    for word in ["left", "lt", "LT", "left.all.all"] {
        assert_eq!(parse(word), result(S::Left, T::All, K::All), "{word}");
    }
    assert_eq!(parse("rt.orphan"), result(S::Right, T::Orphan, K::All));
    assert_eq!(parse("left.diff.files"), result(S::Left, T::Diff, K::Files));
    assert_eq!(parse("diff"), result(S::All, T::Diff, K::All));
    assert_eq!(parse("diff.files"), result(S::All, T::Diff, K::Files));
    for word in ["files", "all.files", "all.all.files"] {
        assert_eq!(parse(word), result(S::All, T::All, K::Files), "{word}");
    }
    assert_eq!(parse("newer.folders"), result(S::All, T::Newer, K::Folders));
    assert_eq!(parse("Empty.Folders"), SelectMask::EmptyFolders);
    for word in [
        "left.left",
        "files.left",
        "bogus",
        "",
        "left.",
        "all.left",
        "all.all.all.all",
        "empty",
    ] {
        match SelectMask::parse(word) {
            Err(FolderError::InvalidRules(text)) => assert!(text.contains(word), "{text}"),
            other => panic!("{word}: {other:?}"),
        }
    }
    // W8's examples.
    assert!(SelectMask::parse("left.diff.files").is_ok());
    assert!(SelectMask::parse("left.orphan.files").is_ok());
}

#[test]
fn a6_14_display_is_the_canonical_word() {
    for word in [
        "left.diff.files",
        "all.all.all",
        "right.older.folders",
        "empty.folders",
    ] {
        let mask = SelectMask::parse(word).unwrap();
        assert_eq!(mask.to_string(), word);
        assert_eq!(SelectMask::parse(&mask.to_string()).unwrap(), mask);
    }
}

#[test]
fn a6_14_matching_table_files() {
    let cmp = compared(&tree(), &FolderRules::default());
    assert_eq!(
        picked(&cmp, "newer.files"),
        ["mixed/other:l", "newer:l", "older:r", "timeonly:l"]
    );
    assert_eq!(
        picked(&cmp, "older.files"),
        ["mixed/other:r", "newer:r", "older:l", "timeonly:r"]
    );
    assert_eq!(picked(&cmp, "left.older.files"), ["older:l"]);
    assert_eq!(
        picked(&cmp, "orphan.files"),
        ["lfolder/f:l", "lonly:l", "ronly:r"]
    );
    assert_eq!(
        picked(&cmp, "left.orphan.files"),
        ["lfolder/f:l", "lonly:l"]
    );
    // `diff` is differences other than the time.
    let diff = picked(&cmp, "diff.files");
    assert!(diff.contains(&"newer:lr".to_owned()), "{diff:?}");
    assert!(diff.contains(&"diffsame:lr".to_owned()), "{diff:?}");
    assert!(
        !diff.iter().any(|row| row.starts_with("timeonly")),
        "{diff:?}"
    );
    assert!(!diff.iter().any(|row| row.starts_with("same")), "{diff:?}");
    let exact = picked(&cmp, "exact.files");
    assert!(exact.contains(&"same:lr".to_owned()));
    assert!(exact.contains(&"link:lr".to_owned()));
    assert!(!exact.contains(&"lonly:l".to_owned()));
}

#[test]
fn a6_14_folders_match_only_if_their_contents_do() {
    let cmp = compared(&tree(), &FolderRules::default());
    let exact = picked(&cmp, "exact.folders");
    assert!(exact.contains(&"alls:lr".to_owned()), "{exact:?}");
    assert!(exact.contains(&"emptyboth:lr".to_owned()), "{exact:?}");
    // One child that does not match.
    assert!(
        !exact.iter().any(|row| row.starts_with("mixed")),
        "{exact:?}"
    );
    let orphans = picked(&cmp, "orphan.folders");
    assert!(orphans.contains(&"lfolder:l".to_owned()), "{orphans:?}");
    assert!(orphans.contains(&"emptyleft:l".to_owned()), "{orphans:?}");
    // An empty folder is no `newer`, nor `diff`.
    assert_eq!(picked(&cmp, "newer.folders"), Vec::<String>::new());
    let everything = picked(&cmp, "all.folders");
    assert!(
        everything.contains(&"mixed:lr".to_owned()),
        "{everything:?}"
    );
    assert!(everything.contains(&"emptyboth:lr".to_owned()));
}

#[test]
fn a6_14_empty_folders() {
    let cmp = compared(&tree(), &FolderRules::default());
    let found = picked(&cmp, "empty.folders");
    assert!(found.contains(&"emptyboth:lr".to_owned()), "{found:?}");
    assert!(found.contains(&"emptyleft:l".to_owned()), "{found:?}");
    assert!(found.contains(&"hollow:l".to_owned()), "{found:?}");
    assert!(found.contains(&"hollow/sub:l".to_owned()), "{found:?}");
    assert!(
        !found.iter().any(|row| row.starts_with("alls")),
        "{found:?}"
    );
}

#[test]
fn a6_14_masks_are_ored_and_add_to_what_is_there() {
    let cmp = compared(&tree(), &FolderRules::default());
    let read = cmp.read();
    let shown = all();
    let scope = SelectScope {
        display: &shown,
        expanded: None,
    };
    let masks = [
        SelectMask::parse("left.orphan.files").unwrap(),
        SelectMask::parse("right.orphan.files").unwrap(),
    ];
    let mut sel = Selection::default();
    sel.insert(id(&read, "same"), SideSet::LEFT);
    select(&read, &scope, &masks, false, &mut sel);
    assert_eq!(
        words(&read, &sel),
        ["lfolder/f:l", "lonly:l", "ronly:r", "same:l"]
    );
}

#[test]
fn a6_14_a_collapsed_folder_is_one_row() {
    let cmp = compared(&tree(), &FolderRules::default());
    let read = cmp.read();
    let shown = all();
    let closed = |_: NodeId| false;
    let scope = SelectScope {
        display: &shown,
        expanded: Some(&closed),
    };
    let mut sel = Selection::default();
    select(
        &read,
        &scope,
        &[SelectMask::parse("exact").unwrap()],
        false,
        &mut sel,
    );
    let found = words(&read, &sel);
    assert!(found.contains(&"alls:lr".to_owned()), "{found:?}");
    assert!(!found.contains(&"alls/a:lr".to_owned()), "{found:?}");
    // Its match still reads what is below it.
    assert!(
        !found.iter().any(|row| row.starts_with("mixed")),
        "{found:?}"
    );
}

#[test]
fn a6_14_all_files_invert_restrict() {
    let cmp = compared(&tree(), &FolderRules::default());
    let read = cmp.read();
    let shown = display(FolderDisplayFilter::All);
    let scope = SelectScope {
        display: &shown,
        expanded: None,
    };
    let mut sel = Selection::default();
    select_all_files(&read, &scope, &mut sel);
    assert!(sel.get(id(&read, "same")) == SideSet::all());
    assert_eq!(sel.get(id(&read, "lonly")), SideSet::LEFT);
    assert_eq!(sel.get(id(&read, "alls")), SideSet::empty());
    let before = sel.clone();
    sel.invert(&read, &scope);
    assert_eq!(sel.get(id(&read, "same")), SideSet::empty());
    assert_eq!(sel.get(id(&read, "alls")), SideSet::all());
    sel.invert(&read, &scope);
    assert_eq!(sel, before);
    sel.restrict(Side::Left);
    assert_eq!(sel.get(id(&read, "same")), SideSet::LEFT);
    assert_eq!(sel.get(id(&read, "ronly")), SideSet::empty());
    sel.remove(id(&read, "same"), SideSet::LEFT);
    assert_eq!(sel.get(id(&read, "same")), SideSet::empty());
    assert!(!sel.is_empty());
}

#[test]
fn a6_14_iu_reads_the_category_of_the_mask() {
    // An unimportant-only pair is `same` under `iu`, and a difference without.
    let vfs = bases();
    file(&vfs, "/l/a.txt", b"x", 10);
    file(&vfs, "/r/a.txt", b"x", 10);
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    let read = cmp.read();
    let shown = all();
    let scope = SelectScope {
        display: &shown,
        expanded: None,
    };
    for iu in [false, true] {
        let mut sel = Selection::default();
        select(
            &read,
            &scope,
            &[SelectMask::parse("exact").unwrap()],
            iu,
            &mut sel,
        );
        assert_eq!(words(&read, &sel), ["a.txt:lr"]);
    }
}

fn filtered_rules() -> FolderRules {
    FolderRules {
        filters: FilterRules {
            names: NameFilterLists {
                exclude_files: vec!["*.bak".to_owned()],
                ..NameFilterLists::default()
            },
            ..FilterRules::default()
        },
        ..FolderRules::default()
    }
}

fn include(hidden: bool, filter: FolderDisplayFilter) -> Include {
    Include {
        hidden,
        display: display(filter),
    }
}

/// `<path>:<sides>:<whole sides>`.
fn rows(read: &TreeRead<'_>, sel: &Selection, inc: &Include) -> Vec<String> {
    let code = |set: SideSet| match (set.contains(SideSet::LEFT), set.contains(SideSet::RIGHT)) {
        (true, true) => "lr",
        (true, false) => "l",
        (false, true) => "r",
        (false, false) => "-",
    };
    resolve(read, sel, inc)
        .iter()
        .map(|row| {
            format!(
                "{}:{}:{}",
                read.path(row.node).as_str(),
                code(row.sides),
                code(row.whole)
            )
        })
        .collect()
}

#[test]
fn a6_14_resolve_a_folder_gives_its_rows_in_preorder() {
    let vfs = bases();
    file(&vfs, "/l/a/b/x", b"x", 10);
    file(&vfs, "/l/a/y", b"y", 10);
    file(&vfs, "/r/a/y", b"y", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a:l:l", "a/b:l:l", "a/b/x:l:l", "a/y:l:l"]
    );
    let depths: Vec<u32> = resolve(&read, &sel, &include(false, FolderDisplayFilter::All))
        .iter()
        .map(|row| row.depth)
        .collect();
    assert_eq!(depths, [0, 1, 2, 1]);
}

#[test]
fn a6_14_resolve_a_folder_and_a_child_counts_the_rows_below() {
    let vfs = bases();
    file(&vfs, "/l/a/b/x", b"x", 10);
    file(&vfs, "/l/a/y", b"y", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::LEFT);
    sel.insert(id(&read, "a/b/x"), SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a/b/x:l:l"]
    );
    // On the other side the folder is not affected by what is selected on this one.
    let vfs = bases();
    file(&vfs, "/l/a/w", b"x", 10);
    file(&vfs, "/l/a/x", b"x", 10);
    file(&vfs, "/r/a/x", b"x", 10);
    file(&vfs, "/r/a/z", b"z", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::all());
    sel.insert(id(&read, "a/x"), SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a:r:r", "a/x:lr:lr", "a/z:r:r"]
    );
}

#[test]
fn a6_14_resolve_a_folder_with_all_it_shows_selected_is_the_folder() {
    // `select all` picks a folder and each row below it: that is no choice among its rows.
    let vfs = bases();
    file(&vfs, "/l/a/b/x", b"x", 10);
    file(&vfs, "/l/a/y", b"y", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let shown = all();
    let scope = SelectScope {
        display: &shown,
        expanded: None,
    };
    let mut sel = Selection::default();
    select(
        &read,
        &scope,
        &[SelectMask::parse("all").unwrap()],
        false,
        &mut sel,
    );
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a:l:l", "a/b:l:l", "a/b/x:l:l", "a/y:l:l"]
    );
    // One row short of it, the rows count.
    sel.remove(id(&read, "a/y"), SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a/b:l:l", "a/b/x:l:l"]
    );
}

#[test]
fn a6_14_resolve_a_hidden_row_is_left_out_and_ends_whole() {
    let vfs = bases();
    file(&vfs, "/l/a/same", b"s", 10);
    file(&vfs, "/r/a/same", b"s", 10);
    file(&vfs, "/l/a/only", b"o", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::LEFT);
    assert_eq!(
        rows(
            &read,
            &sel,
            &include(false, FolderDisplayFilter::Differences)
        ),
        ["a:l:-", "a/only:l:l"]
    );
    assert_eq!(
        rows(
            &read,
            &sel,
            &include(true, FolderDisplayFilter::Differences)
        ),
        ["a:l:l", "a/only:l:l", "a/same:l:l"]
    );
}

#[test]
fn a6_14_resolve_filtered_rows_with_and_without_hidden() {
    let vfs = bases();
    file(&vfs, "/l/a/x.txt", b"x", 10);
    file(&vfs, "/l/a/x.bak", b"b", 10);
    let cmp = compared(&vfs, &filtered_rules());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::LEFT);
    // A file filter hides the row whatever the display shows.
    let mut suppressed = include(false, FolderDisplayFilter::All);
    suppressed.display.suppress_filters = true;
    for inc in [include(false, FolderDisplayFilter::All), suppressed] {
        assert_eq!(rows(&read, &sel, &inc), ["a:l:-", "a/x.txt:l:l"]);
    }
    assert_eq!(
        rows(&read, &sel, &include(true, FolderDisplayFilter::All)),
        ["a:l:l", "a/x.bak:l:l", "a/x.txt:l:l"]
    );
    // A row named by itself counts: its operation says what becomes of it.
    let mut sel = Selection::default();
    sel.insert(id(&read, "a/x.bak"), SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a/x.bak:l:l"]
    );
}

#[test]
fn a6_14_resolve_whole_off_for_a_folder_not_scanned() {
    let vfs = bases();
    file(&vfs, "/l/a/sub/x", b"x", 10);
    file(&vfs, "/r/a/y", b"y", 10);
    file(&vfs, "/l/other", b"x", 10);
    let mut rules = FolderRules::default();
    rules.handling.auto_scan_subfolders = false;
    let cmp = compared(&vfs, &rules);
    let read = cmp.read();
    let a = id(&read, "a");
    assert_eq!(read.node(a).unwrap().scan, ScanStatus::NotScanned);
    let mut sel = Selection::default();
    sel.insert(a, SideSet::LEFT);
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["a:l:-"]
    );
}

#[test]
fn a6_14_resolve_whole_off_for_an_error_row() {
    let vfs = bases();
    file(&vfs, "/l/a/good", b"g", 10);
    file(&vfs, "/l/a/sub/x", b"x", 10);
    file(&vfs, "/r/a/sub/x", b"y", 10);
    vfs.inject(hikasa_vfs::Fault {
        path: host("/r/a/sub"),
        op: hikasa_vfs::VfsOp::List,
        kind: hikasa_core::ErrorKind::PermissionDenied,
    });
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(id(&read, "a"), SideSet::LEFT);
    let found = rows(&read, &sel, &include(false, FolderDisplayFilter::All));
    assert!(found.contains(&"a:l:-".to_owned()), "{found:?}");
    assert!(found.contains(&"a/sub:l:-".to_owned()), "{found:?}");
    assert!(found.contains(&"a/good:l:l".to_owned()), "{found:?}");
}

#[test]
fn a6_14_resolve_the_base_selected_gives_its_rows_and_no_root() {
    let vfs = bases();
    file(&vfs, "/l/x", b"x", 10);
    file(&vfs, "/r/y", b"y", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut sel = Selection::default();
    sel.insert(NodeId::ROOT, SideSet::all());
    assert_eq!(
        rows(&read, &sel, &include(false, FolderDisplayFilter::All)),
        ["x:l:l", "y:r:r"]
    );
}

#[test]
fn a6_14_open_pair_table() {
    let cmp = compared(&tree(), &FolderRules::default());
    let read = cmp.read();
    let spec = |path: &str, side| read.spec(id(&read, path), side).unwrap();
    // One item with both sides.
    let mut sel = Selection::default();
    sel.insert(id(&read, "same"), SideSet::all());
    let pair = open_pair(&read, &sel, None).unwrap();
    assert_eq!(pair.left, spec("same", Side::Left));
    assert_eq!(pair.right, spec("same", Side::Right));
    // Two items anywhere, in tree order.
    let mut sel = Selection::default();
    sel.insert(id(&read, "same"), SideSet::RIGHT);
    sel.insert(id(&read, "alls/a"), SideSet::LEFT);
    let pair = open_pair(&read, &sel, None).unwrap();
    assert_eq!(pair.left, spec("alls/a", Side::Left));
    assert_eq!(pair.right, spec("same", Side::Right));
    // One item and a mark.
    let mut sel = Selection::default();
    sel.insert(id(&read, "same"), SideSet::LEFT);
    assert!(open_pair(&read, &sel, None).is_none());
    let mark = Some((id(&read, "lonly"), Side::Left));
    let pair = open_pair(&read, &sel, mark).unwrap();
    assert_eq!(pair.left, spec("lonly", Side::Left));
    assert_eq!(pair.right, spec("same", Side::Left));
    // The mark on the item itself is one item.
    assert!(open_pair(&read, &sel, Some((id(&read, "same"), Side::Left))).is_none());
    // Nothing, or three.
    assert!(open_pair(&read, &Selection::default(), None).is_none());
    let mut sel = Selection::default();
    sel.insert(id(&read, "same"), SideSet::all());
    sel.insert(id(&read, "newer"), SideSet::LEFT);
    assert!(open_pair(&read, &sel, None).is_none());
}

fn query(text: &str) -> FindQuery {
    FindQuery {
        text: text.to_owned(),
        case: false,
        whole_word: false,
        regex: false,
        wrap: false,
        up: false,
        sides: SideSet::all(),
    }
}

#[test]
fn a6_14_finder() {
    let vfs = bases();
    file(&vfs, "/l/Alpha.txt", b"", 10);
    file(&vfs, "/l/beta_alpha", b"", 10);
    file(&vfs, "/r/gamma alpha x", b"", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let mut shown = Vec::new();
    read.visible_rows(&all(), &|_| true, &mut shown);
    assert_eq!(shown.len(), 3);
    let next = |q: &FindQuery, from| Finder::new(q).unwrap().next(&read, &shown, from);
    // Folded substring, from the start and after a row.
    assert_eq!(next(&query("ALPHA"), None), Some(0));
    assert_eq!(next(&query("alpha"), Some(0)), Some(1));
    assert_eq!(next(&query("alpha"), Some(2)), None);
    // Case.
    let mut q = query("alpha");
    q.case = true;
    assert_eq!(next(&q, None), Some(1));
    // Whole words: `_` is part of one.
    let mut q = query("alpha");
    q.whole_word = true;
    assert_eq!(next(&q, None), Some(0));
    assert_eq!(next(&q, Some(0)), Some(2));
    // Wrap, up.
    let mut q = query("alpha");
    q.wrap = true;
    assert_eq!(next(&q, Some(2)), Some(0));
    let mut q = query("alpha");
    q.up = true;
    assert_eq!(next(&q, None), Some(2));
    assert_eq!(next(&q, Some(1)), Some(0));
    assert_eq!(next(&q, Some(0)), None);
    q.wrap = true;
    assert_eq!(next(&q, Some(0)), Some(2));
    // Sides.
    let mut q = query("gamma");
    q.sides = SideSet::LEFT;
    assert_eq!(next(&q, None), None);
    q.sides = SideSet::RIGHT;
    assert_eq!(next(&q, None), Some(2));
    // A regular expression; not matching; a bad one.
    let mut q = query("^beta_.*a$");
    q.regex = true;
    assert_eq!(next(&q, None), Some(1));
    let mut q = query("nothing");
    q.wrap = true;
    assert_eq!(next(&q, Some(1)), None);
    let mut q = query("(");
    q.regex = true;
    assert_eq!(
        hikasa_core::Classify::kind(&Finder::new(&q).unwrap_err()),
        hikasa_core::ErrorKind::InvalidInput
    );
}

#[test]
fn a6_14_diff_with_an_override_is_no_difference() {
    // A time difference a content result cancels reads as `same`: `diff` follows the category.
    let vfs = bases();
    file(&vfs, "/l/a", b"same", 20);
    file(&vfs, "/r/a", b"same", 10);
    let mut rules = FolderRules::default();
    rules.criteria = Criteria {
        content: hikasa_folder::ContentRule {
            method: hikasa_core::ContentMethod::Binary,
            skip_if_quick_same: false,
            override_quick: true,
            ..rules.criteria.content
        },
        ..rules.criteria
    };
    let cmp = compared(&vfs, &rules);
    assert_eq!(picked(&cmp, "diff"), Vec::<String>::new());
    assert_eq!(picked(&cmp, "exact.files"), ["a:lr"]);
}

// ---- A6.22: the masks of Exclude --------------------------------------------------------------------------------

use hikasa_core::filter::NameFilter;
use hikasa_core::mask::{CaseSensitivity as MaskCase, Mask, MaskOptions};
use hikasa_folder::select::{ExcludeWhat, escape_mask, exclude_masks};
use proptest::prelude::*;

fn selected(read: &TreeRead<'_>, paths: &[&str]) -> Selection {
    let mut sel = Selection::default();
    for path in paths {
        let row = id(read, path);
        let sides = if read.node(row).unwrap().left.is_some() {
            SideSet::LEFT
        } else {
            SideSet::RIGHT
        };
        sel.insert(row, sides);
    }
    sel
}

#[test]
fn a6_22_exclude_masks_items() {
    let vfs = bases();
    file(&vfs, "/l/a/b.txt", b"x", 10);
    file(&vfs, "/l/top.txt", b"x", 10);
    vfs.insert_dir(&host("/l/a/deep"), meta(1)).unwrap();
    vfs.insert_dir(&host("/l/top"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let sel = selected(&read, &["a/b.txt", "top.txt", "a/deep", "top"]);
    let (files, folders) = exclude_masks(&read, &sel, ExcludeWhat::Items);
    assert_eq!(files, [".\\top.txt", "a\\b.txt"]);
    assert_eq!(folders, [".\\top\\", "a\\deep\\"]);
}

#[test]
fn a6_22_exclude_masks_type_without_an_extension_and_nothing_twice() {
    let vfs = bases();
    file(&vfs, "/l/one.txt", b"x", 10);
    file(&vfs, "/l/two.txt", b"x", 10);
    file(&vfs, "/l/Makefile", b"x", 10);
    file(&vfs, "/l/a.[x]", b"x", 10);
    vfs.insert_dir(&host("/l/dir"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let sel = selected(&read, &["one.txt", "two.txt", "Makefile", "a.[x]", "dir"]);
    let (files, folders) = exclude_masks(&read, &sel, ExcludeWhat::Type);
    assert_eq!(files, ["*.[[]x]", "*.txt", ".\\Makefile"]);
    assert!(folders.is_empty());
}

#[test]
fn a6_22_a_path_with_syntax_in_it_is_escaped_component_by_component() {
    let vfs = bases();
    file(&vfs, "/l/[d]/*x", b"x", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let sel = selected(&read, &["[d]/*x"]);
    let (files, _) = exclude_masks(&read, &sel, ExcludeWhat::Items);
    assert_eq!(files, ["[[]d]\\[*]x"]);
    let lists = hikasa_core::filter::NameFilterLists {
        exclude_files: files,
        ..Default::default()
    };
    let options = MaskOptions {
        case: MaskCase::Sensitive,
    };
    let filter = NameFilter::from_lists(&lists, options).unwrap();
    let parent = RelPath::parse("[d]").unwrap();
    let verdict = hikasa_core::filter::FolderVerdict::Included;
    assert!(!filter.file(&parent, "*x", verdict));
    assert!(filter.file(&parent, "yx", verdict));
}

fn matches_name(name: &str, other: &str) -> bool {
    let options = MaskOptions {
        case: MaskCase::Sensitive,
    };
    let mask = Mask::parse(&escape_mask(name), options)
        .unwrap_or_else(|e| panic!("{name:?}: {} ({e})", escape_mask(name)));
    mask.matches_path(&RelPath::parse(other).unwrap())
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/select.txt"))]

    /// The escaped mask matches its name and no other of a generated set.
    #[test]
    fn a6_22_escape_mask_round_trips(
        name in "[-a-z\\[\\]?*;! .]{1,10}",
        others in proptest::collection::vec("[-a-z\\[\\]?*;! .]{1,10}", 0..8),
    ) {
        prop_assume!(!name.starts_with(' ') || name.trim() != "");
        prop_assume!(name != "." && name != "..");
        prop_assert!(matches_name(&name, &name), "{}", escape_mask(&name));
        for other in others.iter().filter(|other| *other != &name) {
            prop_assume!(other.as_str() != "." && other.as_str() != "..");
            prop_assert!(!matches_name(&name, other), "{} matches {other}", escape_mask(&name));
        }
    }
}

#[test]
fn a6_22_escape_mask_table() {
    assert_eq!(escape_mask("a[b"), "a[[]b");
    assert_eq!(escape_mask("a?b"), "a[?]b");
    assert_eq!(escape_mask("a*b"), "a[*]b");
    assert_eq!(escape_mask("a;b"), "a[;]b");
    assert_eq!(escape_mask("-a"), "[-]a");
    assert_eq!(escape_mask(" a"), "[ ]a");
    assert_eq!(escape_mask("a "), "a[ ]");
    assert_eq!(escape_mask("!a"), "!a");
    assert_eq!(escape_mask("a."), "a[.]");
    assert_eq!(escape_mask("plain.txt"), "plain.txt");
}
