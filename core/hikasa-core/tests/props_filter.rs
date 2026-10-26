//! NameFilter properties P-FILTER-1 (exclusion is monotone) and P-FILTER-2 (Excluded propagates), A1.3.

mod common;

use hikasa_core::filter::{FolderVerdict, NameFilter};
use hikasa_core::mask::{CaseSensitivity, Mask, MaskOptions, MaskTarget, Polarity};
use hikasa_core::path::RelPath;
use proptest::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Folder,
}

/// Masks typical of filters: simple, relative, anchored and `...` masks, files and folders; the body of an
/// exclude item may start with blanks (`- a` excludes the name ` a`, §2.2.1).
fn filter_item() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec![
        "a", "b", "x", "*", "*.c", "a*", "?", "[ab]", "*.", "src", "x.*",
    ]);
    (
        any::<bool>(),
        prop::sample::select(vec!["", "", "", " ", "\t", " \t"]),
        prop::sample::select(vec!["", "\\", ".\\", "...\\"]),
        proptest::collection::vec(name, 1..3),
        any::<bool>(),
    )
        .prop_map(|(exclude, blanks, lead, parts, folder)| {
            let mut text = String::new();
            if exclude {
                text.push('-');
            }
            text.push_str(blanks);
            text.push_str(lead);
            text.push_str(&parts.join("\\"));
            if folder {
                text.push('\\');
            }
            text
        })
}

fn tree() -> impl Strategy<Value = Vec<(Vec<String>, Kind)>> {
    let component = prop::sample::select(vec![
        "a", "b", "x", "src", "a.c", "b.c", "x.h", "ab", "Makefile", " a", "\ta", " \tx", " src",
    ]);
    proptest::collection::vec(
        (
            proptest::collection::vec(component.prop_map(str::to_owned), 1..5),
            prop_oneof![Just(Kind::File), Just(Kind::Folder)],
        ),
        1..24,
    )
}

/// Ranking of visibility: Excluded < PassThrough < Included/Base.
fn rank(v: FolderVerdict) -> u8 {
    match v {
        FolderVerdict::Excluded => 0,
        FolderVerdict::PassThrough => 1,
        FolderVerdict::Included | FolderVerdict::Base => 2,
    }
}

/// Folder verdicts along `path` (every component treated as a folder) and the verdict of the item itself:
/// `rank` for folders, 2/0 for included/excluded files.
fn evaluate(filter: &NameFilter, path: &[String], kind: Kind) -> (Vec<FolderVerdict>, u8) {
    let mut parent = RelPath::root();
    let mut verdict = FolderVerdict::Base;
    let mut chain = Vec::new();
    let (name, folders) = path.split_last().unwrap();
    for folder in folders {
        verdict = filter.folder(&parent, folder, verdict);
        chain.push(verdict);
        parent = parent.join(folder).unwrap();
    }
    let item = match kind {
        Kind::Folder => rank(filter.folder(&parent, name, verdict)),
        Kind::File => {
            if filter.file(&parent, name, verdict) {
                2
            } else {
                0
            }
        }
    };
    (chain, item)
}

const OPTS: MaskOptions = MaskOptions {
    case: CaseSensitivity::Insensitive,
};

/// §2.3 written out directly over the single masks (`Mask::matches`, `Mask::may_match_below`), as the reference
/// for `NameFilter`'s list evaluation (which skips masks by their literal first and last bytes).
struct Reference {
    include_files: Vec<Mask>,
    exclude_files: Vec<Mask>,
    include_folders: Vec<Mask>,
    exclude_folders: Vec<Mask>,
}

impl Reference {
    fn parse(filter: &str, opts: MaskOptions) -> Option<Reference> {
        let mut r = Reference {
            include_files: Vec::new(),
            exclude_files: Vec::new(),
            include_folders: Vec::new(),
            exclude_folders: Vec::new(),
        };
        for item in filter.split(';') {
            if item.trim_matches([' ', '\t']).is_empty() {
                continue;
            }
            let mask = Mask::parse(item, opts).ok()?;
            match (mask.polarity(), mask.target()) {
                (Polarity::Include, MaskTarget::Files) => r.include_files.push(mask),
                (Polarity::Exclude, MaskTarget::Files) => r.exclude_files.push(mask),
                (Polarity::Include, MaskTarget::Folders) => r.include_folders.push(mask),
                (Polarity::Exclude, MaskTarget::Folders) => r.exclude_folders.push(mask),
            }
        }
        Some(r)
    }

    /// §2.3 `folder`, steps 1–7.
    fn folder(&self, p: &RelPath, n: &str, parent: FolderVerdict) -> FolderVerdict {
        if parent == FolderVerdict::Excluded {
            return FolderVerdict::Excluded;
        }
        if self.exclude_folders.iter().any(|m| m.matches(p, n)) {
            return FolderVerdict::Excluded;
        }
        if self.include_folders.is_empty() {
            return FolderVerdict::Included;
        }
        if parent == FolderVerdict::Included {
            return FolderVerdict::Included;
        }
        if self.include_folders.iter().any(|m| m.matches(p, n)) {
            return FolderVerdict::Included;
        }
        // `may_match_below` is true for unanchored masks (step 6: "m is unanchored, or …").
        if self.include_folders.iter().any(|m| m.may_match_below(p, n)) {
            return FolderVerdict::PassThrough;
        }
        FolderVerdict::Excluded
    }

    /// §2.3 `file`.
    fn file(&self, p: &RelPath, n: &str, parent: FolderVerdict) -> bool {
        !matches!(parent, FolderVerdict::Excluded | FolderVerdict::PassThrough)
            && !self.exclude_files.iter().any(|m| m.matches(p, n))
            && (self.include_files.is_empty() || self.include_files.iter().any(|m| m.matches(p, n)))
    }
}

/// Filter items with leading and trailing literals in both cases (the list prefilter compares them).
fn literal_item() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec![
        "*.c", "*.C", "A*", "a*", "ab", "AB", "*b", "x?.H", "[a-c]*z", "*.", "*.*", "a*.",
        "README*", "*~",
    ]);
    (
        any::<bool>(),
        prop::sample::select(vec!["", "\\", ".\\", "...\\", "src\\", "SRC\\...\\"]),
        name,
        any::<bool>(),
    )
        .prop_map(|(exclude, lead, name, folder)| {
            let mut text = String::new();
            if exclude {
                text.push('-');
            }
            text.push_str(lead);
            text.push_str(name);
            if folder {
                text.push('\\');
            }
            text
        })
}

fn literal_tree() -> impl Strategy<Value = Vec<(Vec<String>, Kind)>> {
    let component = prop::sample::select(vec![
        "a",
        "A",
        "ab",
        "AB",
        "aB.c",
        "x.C",
        "xY.h",
        "bz",
        "abz",
        "README",
        "readme.md",
        "a.",
        "x~",
        "src",
        "SRC",
        "é.c",
        "",
        "c",
    ]);
    proptest::collection::vec(
        (
            proptest::collection::vec(component.prop_map(str::to_owned), 1..4),
            prop_oneof![Just(Kind::File), Just(Kind::Folder)],
        ),
        1..16,
    )
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_filter.txt"))]

    /// P-FILTER-1: adding an exclude mask never adds included items (and never raises a folder's verdict).
    #[test]
    fn p_filter_1_exclusion_is_monotone(
        items in proptest::collection::vec(filter_item(), 0..5),
        extra in filter_item(),
        exclude_folder in any::<bool>(),
        tree in tree(),
    ) {
        let Ok(before) = NameFilter::parse(&items.join(";"), OPTS) else { return Ok(()); };
        let mut extra = extra.trim_start_matches('-').to_owned();
        if exclude_folder && !extra.ends_with('\\') {
            extra.push('\\');
        }
        let Ok(after) = NameFilter::parse(&format!("{};-{}", items.join(";"), extra), OPTS) else { return Ok(()); };
        for (path, kind) in &tree {
            let (chain_before, item_before) = evaluate(&before, path, *kind);
            let (chain_after, item_after) = evaluate(&after, path, *kind);
            prop_assert!(item_after <= item_before, "{:?} {:?} became more visible with -{}", kind, path, extra);
            for (b, a) in chain_before.iter().zip(&chain_after) {
                prop_assert!(rank(*a) <= rank(*b), "folder verdict rose on {:?}", path);
            }
        }
    }

    /// `NameFilter` (whose lists skip masks by their literal first and last bytes) agrees with §2.3 evaluated
    /// mask by mask, in both case modes and for every parent verdict.
    #[test]
    fn name_filter_equals_the_reference(
        items in proptest::collection::vec(prop_oneof![literal_item(), filter_item()], 0..8),
        tree in literal_tree(),
        insensitive in any::<bool>(),
    ) {
        let opts = MaskOptions {
            case: if insensitive { CaseSensitivity::Insensitive } else { CaseSensitivity::Sensitive },
        };
        let text = items.join(";");
        let (Ok(filter), Some(reference)) = (NameFilter::parse(&text, opts), Reference::parse(&text, opts)) else {
            return Ok(());
        };
        for (path, _) in &tree {
            let (name, folders) = path.split_last().unwrap();
            if folders.iter().any(String::is_empty) {
                continue;
            }
            let parent = folders.iter().fold(RelPath::root(), |p, c| p.join(c).unwrap());
            for verdict in [FolderVerdict::Base, FolderVerdict::Included, FolderVerdict::PassThrough, FolderVerdict::Excluded] {
                prop_assert_eq!(filter.folder(&parent, name, verdict), reference.folder(&parent, name, verdict), "folder {:?} in {:?}", name, parent);
                prop_assert_eq!(filter.file(&parent, name, verdict), reference.file(&parent, name, verdict), "file {:?} in {:?}", name, parent);
            }
        }
    }

    /// P-FILTER-2: `Excluded` propagates to every descendant.
    #[test]
    fn p_filter_2_excluded_propagates(
        items in proptest::collection::vec(filter_item(), 0..5),
        tree in tree(),
    ) {
        let Ok(filter) = NameFilter::parse(&items.join(";"), OPTS) else { return Ok(()); };
        for (path, kind) in &tree {
            let (chain, item) = evaluate(&filter, path, *kind);
            if let Some(first) = chain.iter().position(|v| *v == FolderVerdict::Excluded) {
                prop_assert!(chain[first..].iter().all(|v| *v == FolderVerdict::Excluded), "{:?}: {:?}", path, chain);
                prop_assert_eq!(item, 0, "{:?} below an excluded folder is visible", path);
            }
            // A file below a PassThrough folder is excluded as well.
            if *kind == Kind::File && chain.last() == Some(&FolderVerdict::PassThrough) {
                prop_assert_eq!(item, 0);
            }
        }
        // The round trip of the toolbar syntax keeps the verdicts (INV-MASK-2).
        let again = NameFilter::parse(&filter.to_filter_string(), OPTS).unwrap();
        prop_assert_eq!(again.to_lists(), filter.to_lists());
        // The four lists are lossless as well (§3.3 sends `--filters` through them).
        let from_lists = NameFilter::from_lists(&filter.to_lists(), OPTS).unwrap();
        prop_assert_eq!(from_lists.to_lists(), filter.to_lists());
        for (path, kind) in &tree {
            prop_assert_eq!(evaluate(&again, path, *kind), evaluate(&filter, path, *kind));
            prop_assert_eq!(evaluate(&from_lists, path, *kind), evaluate(&filter, path, *kind), "{:?} via lists", path);
        }
    }
}
