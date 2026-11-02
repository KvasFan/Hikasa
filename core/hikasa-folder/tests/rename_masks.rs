//! M2 W6b acceptance A6.18 (D-180): rename masks by the normative tables, regular expressions, the collision table, chains and
//! cycles on `MemoryVfs` and a temporary folder.

mod common;

use common::{compared, config, engines, host, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, RelPath, Side, Spec};
use hikasa_folder::ops::{
    FolderOp, OpPlan, OpScope, OpSettings, ReadOnlySides, RegexRename, RenameMask, base_mask,
    dos_rename, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{CaseMode, FolderComparison, FolderError, FolderModel, FolderRules, compare_into};
use hikasa_formats::{EngineKind, Pattern, RegexLimits};
use hikasa_vfs::ops::{self, Action, ItemStatus, OpReport, SkipReason};
use hikasa_vfs::{Fault, MemMeta, MemoryVfs, Vfs, VfsOp};
use proptest::prelude::*;

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn tree(case: CaseSensitivity, files: &[(&str, &[u8])]) -> MemoryVfs {
    let vfs = MemoryVfs::new(case);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    for (path, content) in files {
        vfs.insert_file(&host(path), content.to_vec(), meta(10))
            .unwrap();
    }
    vfs
}

fn insensitive() -> FolderRules {
    let mut rules = FolderRules::default();
    rules.alignment.case = CaseMode::Insensitive;
    rules
}

fn rename_plan(
    vfs: &dyn Vfs,
    cmp: &FolderComparison,
    rows: &[&str],
    mask: RenameMask,
) -> Result<OpPlan, FolderError> {
    let read = cmp.read();
    let mut selection = Selection::default();
    for row in rows {
        selection.insert(
            read.node_at(&RelPath::parse(row).unwrap()).unwrap(),
            SideSet::LEFT,
        );
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let op = FolderOp::Rename {
        sides: SideSet::LEFT,
        mask,
    };
    plan_op(&read, vfs, &scope, &op, &OpSettings::default(), &Ctl::detached())
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

fn name_of(spec: &Spec) -> String {
    spec.file_name().unwrap()
}

/// `<from> > <to>` of each item; a skip or an error says so.
fn steps(plan: &OpPlan) -> Vec<String> {
    plan.plan
        .items
        .iter()
        .map(|item| {
            let from = item.source.as_ref().map_or_else(String::new, name_of);
            let to = name_of(&item.target);
            match (&item.skip, &item.error) {
                (Some(reason), _) => format!("{to} !{}", reason.as_str()),
                (_, Some(error)) => format!("{from} > {to} !{}", error.kind.as_str()),
                _ => format!("{from} > {to}"),
            }
        })
        .collect()
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn names_on_left(vfs: &MemoryVfs) -> Vec<String> {
    let mut names: Vec<String> = vfs
        .dump()
        .iter()
        .map(|entry| entry.path.replace('\\', "/"))
        .filter_map(|path| {
            let path = path.strip_prefix("C:").unwrap_or(&path).to_owned();
            path.strip_prefix("/l/").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a6_18_every_row_of_the_dos_table() {
    let table = [
        ("abc1.txt", "abc?.bak", "abc1.bak"),
        ("abcd.txt", "x?cd.*", "xbcd.txt"),
        ("report.txt", "*_old.*", "report_old.txt"),
        ("a.tar.gz", "*.zip", "a.tar.zip"),
        ("a.txt", "b", "b"),
        ("a.txt", "b*", "b.txt"),
        ("longname.txt", "?????.*", "longn.txt"),
        ("a.txt", "*.", "a"),
        ("Makefile", "*.bak", "Makefile.bak"),
        ("Makefile", "*.*", "Makefile"),
        (".bashrc", "*.bak", ".bashrc.bak"),
        ("abc.txt", "?", "a"),
        ("a.txt", "??.t?", "a.tx"),
        ("é.txt", "?x.*", "éx.txt"),
    ];
    for (name, mask, want) in table {
        assert_eq!(dos_rename(name, mask), want, "{name} {mask}");
    }
}

#[test]
fn a6_18_base_mask_table() {
    assert_eq!(base_mask(&["abc1.txt", "abc2.txt"]), "abc?.txt");
    assert_eq!(base_mask(&["a.txt", "bb.txt"]), "?*.txt");
    assert_eq!(base_mask(&["one.txt"]), "one.txt");
    assert_eq!(base_mask(&["Makefile"]), "Makefile");
    assert_eq!(base_mask(&["a.txt", "a"]), "a.*");
    assert_eq!(base_mask(&["a.txt", "a.doc"]), "a.???");
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/rename_masks.txt"))]

    #[test]
    fn a6_18_star_masks_keep_every_name(name in "[a-zA-Zé. _-]{1,12}") {
        prop_assert_eq!(dos_rename(&name, "*.*"), name.clone());
        prop_assert_eq!(dos_rename(&name, "*"), name);
    }
}

#[test]
fn a6_18_regex_groups_names_and_dollars() {
    let apply = |old: &str, new: &str, name: &str| {
        RegexRename::new(old, new).unwrap().apply(name).unwrap()
    };
    assert_eq!(apply(r"(\w+)\.txt", "$1.bak", "abc.txt").as_deref(), Some("abc.bak"));
    assert_eq!(apply(r"(\w+)\.txt", "${1}x", "abc.txt").as_deref(), Some("abcx"));
    assert_eq!(apply(r"(?P<stem>\w+)\.txt", "${stem}!", "abc.txt").as_deref(), Some("abc!"));
    assert_eq!(apply(r"a", "$$", "a").as_deref(), Some("$"));
    // `$1a` is the group, then `a`: the expansion is the same over both engines.
    assert_eq!(apply(r"(b)", "$1a", "b").as_deref(), Some("ba"));
    assert_eq!(apply(r"(b)(?=$)", "$1a", "b").as_deref(), Some("ba"));
    // A group that took no part is empty.
    assert_eq!(apply(r"(x)?b", "[$1]", "b").as_deref(), Some("[]"));
}

#[test]
fn a6_18_regex_a_missing_group_or_a_bad_dollar_is_invalid_input() {
    for (old, new) in [("(a)", "$2"), ("(a)", "${name}"), ("a", "$"), ("a", "$x"), ("(", "x")] {
        let err = RegexRename::new(old, new).unwrap_err();
        assert_eq!(
            hikasa_core::Classify::kind(&err),
            ErrorKind::InvalidInput,
            "{old} {new}"
        );
    }
}

#[test]
fn a6_18_regex_matches_the_whole_name() {
    let rename = RegexRename::new("abc", "x").unwrap();
    assert_eq!(rename.apply("abc").unwrap().as_deref(), Some("x"));
    assert_eq!(rename.apply("abcd").unwrap(), None);
    assert_eq!(rename.apply("zabc").unwrap(), None);
    let either = RegexRename::new("a|ab", "x").unwrap();
    assert_eq!(either.apply("ab").unwrap().as_deref(), Some("x"));
    let folded = RegexRename::new("(?i)ABC", "x").unwrap();
    assert_eq!(folded.apply("abc").unwrap().as_deref(), Some("x"));
    assert_eq!(RegexRename::new("ABC", "x").unwrap().apply("abc").unwrap(), None);
}

#[test]
fn a6_18_a_backreference_goes_through_fancy_regex() {
    let pattern = Pattern::compile(r"^(?:(a)\1)$", false, &RegexLimits::default()).unwrap();
    assert_eq!(pattern.engine(), EngineKind::Fancy);
    let rename = RegexRename::new(r"(a)\1", "$1").unwrap();
    assert_eq!(rename.apply("aa").unwrap().as_deref(), Some("a"));
    assert_eq!(rename.apply("ab").unwrap(), None);
}

#[test]
fn a6_18_a_backtrack_limit_fails_the_item_only() {
    let name = "a".repeat(30);
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/ok", b"1"), (&format!("/l/{name}"), b"2")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let mask = RenameMask::Regex {
        // A backreference keeps it in `fancy-regex`, whose limit it passes on the long name.
        old: r"(a*)*\1b|ok".to_owned(),
        new: "renamed".to_owned(),
    };
    let plan = rename_plan(&vfs, &cmp, &["ok", &name], mask).unwrap();
    let text = steps(&plan);
    assert_eq!(text.len(), 2, "{text:?}");
    assert!(text.contains(&"ok > renamed".to_owned()), "{text:?}");
    assert!(
        text.iter().any(|step| step.ends_with("!invalid_input")),
        "{text:?}"
    );
}

#[test]
fn a6_18_name_checks() {
    let vfs = tree(CaseSensitivity::Sensitive, &[("/l/a", b"1")]);
    let cmp = compared(&vfs, &FolderRules::default());
    for bad in ["", ".", "..", "x/y"] {
        let plan = rename_plan(&vfs, &cmp, &["a"], RenameMask::Name(bad.to_owned())).unwrap();
        assert_eq!(plan.plan.items.len(), 1, "{bad:?}");
        let error = plan.plan.items[0].error.as_ref().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{bad:?}");
    }
    let plan = rename_plan(&vfs, &cmp, &["a"], RenameMask::Name("a".to_owned())).unwrap();
    assert_eq!(plan.plan.items[0].skip, Some(SkipReason::SameName));
    let plan = rename_plan(
        &vfs,
        &cmp,
        &["a"],
        RenameMask::Regex {
            old: "z".to_owned(),
            new: "y".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(plan.plan.items[0].skip, Some(SkipReason::NoMatch));
    // One name needs one row.
    let vfs = tree(CaseSensitivity::Sensitive, &[("/l/a", b"1"), ("/l/b", b"1")]);
    let cmp = compared(&vfs, &FolderRules::default());
    let err = rename_plan(&vfs, &cmp, &["a", "b"], RenameMask::Name("c".to_owned())).unwrap_err();
    assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::InvalidInput);
}

#[cfg(windows)]
#[test]
fn a6_18_reserved_names_fail_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (l, r) = (root.join("l"), root.join("r"));
    std::fs::create_dir(&l).unwrap();
    std::fs::create_dir(&r).unwrap();
    std::fs::write(l.join("a"), b"x").unwrap();
    let vfs = hikasa_vfs::LocalVfs::new(hikasa_vfs::LocalOptions::default());
    let spec = |path: &std::path::Path| {
        Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    let contents = engines(1 << 30);
    let cmp = with_env(&vfs, &contents, &CancelToken::new(), |env| {
        let model = FolderModel::new(spec(&l), spec(&r), FolderRules::default()).unwrap();
        compare_into(&model, env).unwrap()
    });
    for bad in ["CON.txt", "name."] {
        let plan = rename_plan(&vfs, &cmp, &["a"], RenameMask::Name(bad.to_owned())).unwrap();
        let error = plan.plan.items[0].error.as_ref().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{bad}");
    }
}

#[test]
fn a6_18_two_items_reach_one_name() {
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/a1.txt", b"1"), ("/l/a2.txt", b"2")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(
        &vfs,
        &cmp,
        &["a1.txt", "a2.txt"],
        RenameMask::Dos("b.txt".to_owned()),
    )
    .unwrap();
    assert_eq!(
        steps(&plan),
        ["a2.txt > b.txt !already_exists", "a1.txt > b.txt"]
    );
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(read(&vfs, "/l/b.txt").unwrap(), b"1");
    assert_eq!(read(&vfs, "/l/a2.txt").unwrap(), b"2");
}

#[test]
fn a6_18_two_items_reach_one_name_folded_on_a_case_insensitive_back_end() {
    let vfs = tree(
        CaseSensitivity::Insensitive,
        &[("/l/a1", b"1"), ("/l/a2", b"2")],
    );
    let cmp = compared(&vfs, &insensitive());
    let plan = rename_plan(
        &vfs,
        &cmp,
        &["a1", "a2"],
        RenameMask::Regex {
            old: "a1".to_owned(),
            new: "X".to_owned(),
        },
    );
    assert!(plan.is_ok());
    let both = rename_plan(
        &vfs,
        &cmp,
        &["a1", "a2"],
        RenameMask::Regex {
            old: "a(1|2)".to_owned(),
            new: "b".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(steps(&both), ["a2 > b !already_exists", "a1 > b"]);
}

#[test]
fn a6_18_an_existing_name_is_never_replaced() {
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/a", b"1"), ("/l/b", b"2")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["a"], RenameMask::Name("b".to_owned())).unwrap();
    assert_eq!(steps(&plan), ["a > b !already_exists"]);
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(read(&vfs, "/l/b").unwrap(), b"2");
    assert_eq!(read(&vfs, "/l/a").unwrap(), b"1");
}

/// The rotation of a name: `abc` → `bca`, which makes chains and cycles out of one mask.
fn rotate(chars: usize) -> RenameMask {
    let groups: String = (2..=chars).chain(1..=1).map(|n| format!("${n}")).collect();
    RenameMask::Regex {
        old: "(.)".repeat(chars),
        new: groups,
    }
}

fn named(case: CaseSensitivity, names: &[&str]) -> MemoryVfs {
    let files: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|name| (format!("/l/{name}"), name.as_bytes().to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_slice()))
        .collect();
    tree(case, &refs)
}

#[test]
fn a6_18_a_row_that_stays_holds_its_name_for_the_one_behind_it() {
    // `abc → bca`, `bca → cab`, and `cab` stands: `bca` stays, so `abc` cannot go either.
    let vfs = named(CaseSensitivity::Sensitive, &["abc", "bca", "cab"]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["abc", "bca"], rotate(3)).unwrap();
    let text = steps(&plan);
    assert_eq!(text.len(), 2, "{text:?}");
    assert!(text.iter().all(|step| step.ends_with("!already_exists")), "{text:?}");
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 2);
    assert_eq!(names_on_left(&vfs), ["abc", "bca", "cab"]);
}

#[test]
fn a6_18_a_chain_goes_free_end_first() {
    let vfs = named(CaseSensitivity::Sensitive, &["abcd", "bcda", "cdab"]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["abcd", "bcda", "cdab"], rotate(4)).unwrap();
    assert_eq!(
        steps(&plan),
        ["cdab > dabc", "bcda > cdab", "abcd > bcda"]
    );
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(names_on_left(&vfs), ["bcda", "cdab", "dabc"]);
    assert_eq!(read(&vfs, "/l/dabc").unwrap(), b"cdab");
    assert_eq!(read(&vfs, "/l/bcda").unwrap(), b"abcd");
}

#[test]
fn a6_18_a_cycle_of_two_is_broken_by_a_hidden_name() {
    let vfs = named(CaseSensitivity::Sensitive, &["ab", "ba"]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["ab", "ba"], rotate(2)).unwrap();
    assert_eq!(
        steps(&plan),
        ["ab > .ab.hikasa-ren", "ba > ab", ".ab.hikasa-ren > ba"]
    );
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(names_on_left(&vfs), ["ab", "ba"]);
    assert_eq!(read(&vfs, "/l/ab").unwrap(), b"ba");
    assert_eq!(read(&vfs, "/l/ba").unwrap(), b"ab");
}

#[test]
fn a6_18_a_cycle_of_three_ends_as_asked_with_no_hidden_name_left() {
    let vfs = named(CaseSensitivity::Sensitive, &["abc", "bca", "cab"]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["abc", "bca", "cab"], rotate(3)).unwrap();
    assert_eq!(
        steps(&plan),
        [
            "abc > .abc.hikasa-ren",
            "cab > abc",
            "bca > cab",
            ".abc.hikasa-ren > bca"
        ]
    );
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(names_on_left(&vfs), ["abc", "bca", "cab"]);
    assert_eq!(read(&vfs, "/l/bca").unwrap(), b"abc");
    assert_eq!(read(&vfs, "/l/cab").unwrap(), b"bca");
    assert_eq!(read(&vfs, "/l/abc").unwrap(), b"cab");
}

#[test]
fn a6_18_a_hidden_name_that_is_taken_is_numbered() {
    let vfs = named(
        CaseSensitivity::Sensitive,
        &["ab", "ba", ".ab.hikasa-ren"],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["ab", "ba"], rotate(2)).unwrap();
    assert_eq!(steps(&plan)[0], "ab > .ab.hikasa-ren-2");
}

#[test]
fn a6_18_a_name_near_the_limit_breaks_its_cycle_by_a_short_name() {
    let long = "x".repeat(240);
    let (one, two) = (format!("a{long}b"), format!("b{long}a"));
    let vfs = named(CaseSensitivity::Sensitive, &[&one, &two]);
    let cmp = compared(&vfs, &FolderRules::default());
    let mask = RenameMask::Regex {
        old: "(.)(.*)(.)".to_owned(),
        new: "$3$2$1".to_owned(),
    };
    let plan = rename_plan(&vfs, &cmp, &[&one, &two], mask).unwrap();
    assert_eq!(name_of(&plan.plan.items[0].target), ".hikasa-ren");
}

#[test]
fn a6_18_a_fault_in_a_cycles_middle_reports_the_hidden_name() {
    let vfs = named(CaseSensitivity::Sensitive, &["ab", "ba"]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["ab", "ba"], rotate(2)).unwrap();
    vfs.inject(Fault {
        path: host("/l/ba"),
        op: VfsOp::Rename,
        kind: ErrorKind::PermissionDenied,
    });
    let report = run(&vfs, plan);
    assert_eq!(report.results[0].status, ItemStatus::Done);
    assert_eq!(report.results[1].status, ItemStatus::Failed);
    assert_eq!(report.results[2].status, ItemStatus::Failed);
    let last = &report.plan.items[2];
    assert_eq!(name_of(last.source.as_ref().unwrap()), ".ab.hikasa-ren");
    // Nothing is lost: what `ab` held is at the hidden name the report shows.
    assert_eq!(read(&vfs, "/l/.ab.hikasa-ren").unwrap(), b"ab");
    assert_eq!(read(&vfs, "/l/ba").unwrap(), b"ba");
}

#[test]
fn a6_18_case_only_on_a_case_insensitive_back_end_is_one_rename() {
    let vfs = named(CaseSensitivity::Insensitive, &["readme.txt"]);
    let cmp = compared(&vfs, &insensitive());
    let plan = rename_plan(
        &vfs,
        &cmp,
        &["readme.txt"],
        RenameMask::Name("README.TXT".to_owned()),
    )
    .unwrap();
    assert_eq!(steps(&plan), ["readme.txt > README.TXT"]);
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(names_on_left(&vfs), ["README.TXT"]);
}

#[test]
fn a6_18_a_folder_is_one_item_and_its_contents_stay_inside() {
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/d/x", b"1"), ("/l/d/y", b"2")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["d"], RenameMask::Name("e".to_owned())).unwrap();
    assert_eq!(steps(&plan), ["d > e"]);
    assert_eq!(plan.plan.items[0].action, Action::Rename);
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/l/e/y").unwrap(), b"2");
}

#[test]
fn a6_18_a_folder_and_some_of_its_rows_rename_those_alone() {
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/d/x", b"1"), ("/l/d/y", b"2")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["d", "d/x"], RenameMask::Dos("z".to_owned())).unwrap();
    assert_eq!(steps(&plan), ["x > z"]);
}

#[test]
fn a6_18_a_folder_with_all_its_rows_goes_after_them() {
    let vfs = tree(CaseSensitivity::Sensitive, &[("/l/d/x", b"1")]);
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["d", "d/x"], RenameMask::Dos("?z".to_owned())).unwrap();
    assert_eq!(steps(&plan), ["x > xz", "d > dz"]);
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/l/dz/xz").unwrap(), b"1");
}

#[test]
fn a6_18_on_a_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (l, r) = (root.join("l"), root.join("r"));
    std::fs::create_dir(&l).unwrap();
    std::fs::create_dir(&r).unwrap();
    for name in ["abc", "bca", "cab"] {
        std::fs::write(l.join(name), name).unwrap();
    }
    let spec = |path: &std::path::Path| {
        Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    let vfs = hikasa_vfs::LocalVfs::new(hikasa_vfs::LocalOptions::default());
    let contents = engines(1 << 30);
    let cmp = with_env(&vfs, &contents, &CancelToken::new(), |env| {
        let model = FolderModel::new(spec(&l), spec(&r), FolderRules::default()).unwrap();
        compare_into(&model, env).unwrap()
    });
    let plan = rename_plan(&vfs, &cmp, &["abc", "bca", "cab"], rotate(3)).unwrap();
    assert_eq!(plan.plan.items.len(), 4);
    let report = run(&vfs, plan);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(std::fs::read(l.join("bca")).unwrap(), b"abc");
    assert_eq!(std::fs::read(l.join("cab")).unwrap(), b"bca");
    assert_eq!(std::fs::read(l.join("abc")).unwrap(), b"cab");
    assert_eq!(std::fs::read_dir(&l).unwrap().count(), 3);
}

#[test]
fn a6_18_a_rename_on_a_side_of_a_pair_leaves_the_other() {
    let vfs = tree(
        CaseSensitivity::Sensitive,
        &[("/l/a", b"1"), ("/r/a", b"1")],
    );
    let cmp = compared(&vfs, &FolderRules::default());
    let plan = rename_plan(&vfs, &cmp, &["a"], RenameMask::Name("b".to_owned())).unwrap();
    run(&vfs, plan);
    assert_eq!(read(&vfs, "/l/b").unwrap(), b"1");
    assert_eq!(read(&vfs, "/r/a").unwrap(), b"1");
    assert_eq!(Side::Left.opposite(), Side::Right);
}
