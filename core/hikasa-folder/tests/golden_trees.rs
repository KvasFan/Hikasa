//! Tree goldens (§6.2.6): every `tests/fixtures/trees/<case>/{tree.toml, rules.toml, expected.json}` is
//! materialised into a `MemoryVfs`, compared on the current thread, checked against `expected.json`
//! (`CanonicalTree`, statistics, verdicts, warning codes), and — unless the case cancels part-way — compared
//! again with 1- and 8-thread pools and with reversed and shuffled listing orders (T2, INV-VFS-1).

mod common;

use common::*;
use hikasa_folder::FolderRules;
use hikasa_vfs::ListingOrder;

fn load(case: &std::path::Path) -> (TreeFile, FolderRules) {
    let text = std::fs::read_to_string(case.join("tree.toml")).unwrap();
    let tree: TreeFile =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", case.display()));
    let rules_path = case.join("rules.toml");
    let rules = if rules_path.exists() {
        let (rules, warnings) =
            FolderRules::from_toml(&std::fs::read_to_string(rules_path).unwrap()).unwrap();
        assert!(warnings.is_empty(), "{}: {warnings:?}", case.display());
        rules
    } else {
        FolderRules::default()
    };
    (tree, rules)
}

#[test]
fn tree_goldens() {
    let mut cases: Vec<std::path::PathBuf> = std::fs::read_dir(fixtures("trees"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(cases.len() >= 22, "at least 22 tree goldens (§6.5)");
    let pools = [pool(1), pool(8)];
    for case in &cases {
        let (tree, rules) = load(case);
        let vfs = materialize(&tree);
        let cmp = run_tree(&tree, &vfs, &rules, None)
            .unwrap_or_else(|e| panic!("{}: {e}", case.display()));
        cmp.check_invariants()
            .unwrap_or_else(|e| panic!("{}: {e}", case.display()));
        let json = expected_json(&tree.description, &cmp);
        assert_golden(&case.join("expected.json"), json.as_bytes());
        if tree.options.cancel_after_listed.is_some() {
            assert!(!cmp.is_complete(), "{}: cancelled", case.display());
            continue;
        }
        let canonical = cmp.to_canonical();
        for p in &pools {
            let other = run_tree(&tree, &vfs, &rules, p.as_ref()).unwrap();
            other.check_invariants().unwrap();
            assert_eq!(other.to_canonical(), canonical, "{}: pool", case.display());
            assert_eq!(other.stats(false), cmp.stats(false));
            assert_eq!(other.stats(true), cmp.stats(true));
            assert_eq!(
                other.read().warnings(),
                cmp.read().warnings(),
                "{}: warnings",
                case.display()
            );
        }
        for order in [ListingOrder::Reversed, ListingOrder::Shuffled { seed: 11 }] {
            vfs.set_listing_order(order);
            let other = run_tree(&tree, &vfs, &rules, pools[1].as_ref()).unwrap();
            assert_eq!(
                other.to_canonical(),
                canonical,
                "{}: {order:?}",
                case.display()
            );
        }
        vfs.set_listing_order(ListingOrder::Sorted);
    }
}
