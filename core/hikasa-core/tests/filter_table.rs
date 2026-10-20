//! The NameFilter table of §2.3 (rows 1–10; row 11 is a size filter of hikasa-folder).

use hikasa_core::filter::{FolderVerdict, NameFilter, NameFilterLists};
use hikasa_core::mask::{CaseSensitivity, MaskOptions};
use hikasa_core::path::RelPath;
use pretty_assertions::assert_eq;

const OPTS: MaskOptions = MaskOptions {
    case: CaseSensitivity::Sensitive,
};

/// The verdict chain of the folders on the way to `path`, starting from the base (`FolderVerdict::Base`).
fn walk(filter: &NameFilter, folders: &[&str]) -> (RelPath, FolderVerdict) {
    let mut parent = RelPath::root();
    let mut verdict = FolderVerdict::Base;
    for name in folders {
        verdict = filter.folder(&parent, name, verdict);
        parent = parent.join(name).unwrap();
    }
    (parent, verdict)
}

fn folder_verdict(filter: &str, path: &str) -> FolderVerdict {
    let filter = NameFilter::parse(filter, OPTS).unwrap();
    let parts: Vec<&str> = path.split('/').collect();
    let (name, folders) = parts.split_last().unwrap();
    let (parent, verdict) = walk(&filter, folders);
    filter.folder(&parent, name, verdict)
}

fn file_included(filter: &str, path: &str) -> bool {
    let filter = NameFilter::parse(filter, OPTS).unwrap();
    let parts: Vec<&str> = path.split('/').collect();
    let (name, folders) = parts.split_last().unwrap();
    let (parent, verdict) = walk(&filter, folders);
    filter.file(&parent, name, verdict)
}

#[test]
fn row_01_empty_filter_includes_everything() {
    assert!(file_included("", "a.c"));
    assert!(file_included("", "x/y/z.bin"));
    assert_eq!(folder_verdict("", "x/y"), FolderVerdict::Included);
    assert!(NameFilter::parse("", OPTS).unwrap().is_match_all());
}

#[test]
fn row_02_include_files() {
    assert!(file_included("*.c", "a.c"));
    assert!(file_included("*.c", "x/y.c"));
    assert!(!file_included("*.c", "a.h"));
    assert_eq!(folder_verdict("*.c", "x"), FolderVerdict::Included);
}

#[test]
fn row_03_exclude_folder_and_its_subtree() {
    assert_eq!(folder_verdict("-obj\\", "obj"), FolderVerdict::Excluded);
    assert_eq!(folder_verdict("-obj\\", "x/obj"), FolderVerdict::Excluded);
    assert_eq!(
        folder_verdict("-obj\\", "obj/deeper"),
        FolderVerdict::Excluded
    );
    assert!(!file_included("-obj\\", "x/obj/a.c"));
    assert!(file_included("-obj\\", "x/a.c"));
}

#[test]
fn row_04_base_files_are_subject_to_file_masks_only() {
    assert!(file_included("src\\", "top.txt"));
}

#[test]
fn row_05_subtree_rule() {
    assert_eq!(folder_verdict("src\\", "src"), FolderVerdict::Included);
    assert_eq!(folder_verdict("src\\", "src/a"), FolderVerdict::Included);
    assert!(file_included("src\\", "src/a/b.c"));
}

#[test]
fn row_06_simple_include_folder_passes_through() {
    assert_eq!(folder_verdict("src\\", "lib"), FolderVerdict::PassThrough);
    assert!(!file_included("src\\", "lib/x.c"));
    assert_eq!(folder_verdict("src\\", "lib/src"), FolderVerdict::Included);
    assert!(file_included("src\\", "lib/src/x.c"));
}

#[test]
fn row_07_anchored_include_folder() {
    assert_eq!(folder_verdict(".\\src\\", "lib"), FolderVerdict::Excluded);
    assert_eq!(folder_verdict(".\\src\\", "src"), FolderVerdict::Included);
}

#[test]
fn row_08_relative_include_folder() {
    assert_eq!(
        folder_verdict("src\\core\\", "src"),
        FolderVerdict::PassThrough
    );
    assert_eq!(
        folder_verdict("src\\core\\", "src/core"),
        FolderVerdict::Included
    );
    assert!(!file_included("src\\core\\", "src/a.c"));
    assert!(file_included("src\\core\\", "src/core/a.c"));
    assert_eq!(
        folder_verdict("src\\core\\", "src/other"),
        FolderVerdict::Excluded
    );
}

#[test]
fn row_09_excluded_parent() {
    assert!(!file_included("*.c;-test\\", "test/a.c"));
    assert!(file_included("*.c;-test\\", "src/a.c"));
}

#[test]
fn row_10_exclude_files_per_side() {
    // The pair x.bak ↔ x.txt is aligned by hikasa-folder (align_extensions); per side the name filter says:
    assert!(!file_included("-*.bak", "x.bak"), "left side: filtered");
    assert!(file_included("-*.bak", "x.txt"), "right side: visible");
}

#[test]
fn row_11_size_filters_never_use_the_name_prefilter() {
    // `size:>1MB` is an Other Filter of hikasa-folder (stat'ed, then filtered); it adds no name mask, so the
    // name prefilter (D-45) never declines `big.bin` and the entry is stat'ed.
    let names_only = NameFilter::match_all(OPTS);
    assert!(names_only.is_match_all());
    assert!(names_only.file(&RelPath::root(), "big.bin", FolderVerdict::Base));
    assert_eq!(
        names_only.folder(&RelPath::root(), "any", FolderVerdict::Base),
        FolderVerdict::Included
    );
}

#[test]
fn exclusion_always_wins() {
    assert_eq!(
        folder_verdict("src\\;-src\\", "src"),
        FolderVerdict::Excluded
    );
    assert!(!file_included("*.c;-a.c", "a.c"));
}

#[test]
fn lists_round_trip() {
    let lists = NameFilterLists {
        include_files: vec!["*.c".into(), "*.h".into()],
        exclude_files: vec!["*.bak".into()],
        include_folders: vec!["src\\".into()],
        exclude_folders: vec!["obj\\".into()],
    };
    let filter = NameFilter::from_lists(&lists, OPTS).unwrap();
    assert_eq!(filter.to_lists(), lists);
    assert_eq!(filter.to_filter_string(), "*.c;*.h;src\\;-*.bak;-obj\\");
    assert_eq!(filter.options(), OPTS);
    let json = serde_json::to_string(&lists).unwrap();
    assert_eq!(
        serde_json::from_str::<NameFilterLists>(&json).unwrap(),
        lists
    );
    assert_eq!(
        serde_json::from_str::<NameFilterLists>("{}").unwrap(),
        NameFilterLists::default()
    );
}

#[test]
fn filters_are_send_sync_and_cheap_to_clone() {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<NameFilter>();
    let filter = NameFilter::parse("*.c", OPTS).unwrap();
    let clone = filter.clone();
    assert_eq!(clone.to_filter_string(), filter.to_filter_string());
}
