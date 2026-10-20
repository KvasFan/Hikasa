//! The normative mask table of §2.2.4 (37 rows) and the toolbar parsing example.

use hikasa_core::filter::NameFilter;
use hikasa_core::mask::{
    CaseSensitivity, Mask, MaskError, MaskList, MaskOptions, MaskTarget, Polarity,
};
use hikasa_core::path::RelPath;
use pretty_assertions::assert_eq;

const I: CaseSensitivity = CaseSensitivity::Insensitive;
const S: CaseSensitivity = CaseSensitivity::Sensitive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    F,
    D,
}

/// Whether the mask applies to the item: the target decides applicability (file masks never match folders,
/// folder masks never match files), then the full path semantics of §2.2.3.
fn applies(mask: &str, kind: Kind, path: &str, case: CaseSensitivity) -> bool {
    let mask = Mask::parse(mask, MaskOptions { case }).unwrap_or_else(|e| panic!("{mask:?}: {e}"));
    let path = RelPath::parse(path).unwrap();
    let target = match kind {
        Kind::F => MaskTarget::Files,
        Kind::D => MaskTarget::Folders,
    };
    let full = mask.target() == target && mask.matches_path(&path);
    let parent = path.parent().unwrap();
    let split = mask.target() == target && mask.matches(&parent, path.name().unwrap());
    assert_eq!(full, split, "matches_path and matches agree");
    full
}

/// (row, mask, kind, path, case, expected)
const ROWS: &[(u32, &str, Kind, &str, CaseSensitivity, bool)] = &[
    (1, "*.c", Kind::F, "main.c", I, true),
    (2, "*.c", Kind::F, "src/util/x.c", I, true),
    (3, "*.c", Kind::F, "x.C", I, true),
    (4, "*.c", Kind::F, "x.C", S, false),
    (5, "*.c", Kind::D, "lib.c", I, false),
    (6, "?.txt", Kind::F, "a.txt", I, true),
    (6, "?.txt", Kind::F, "ab.txt", I, false),
    (7, "*.", Kind::F, "Makefile", I, true),
    (8, "*.", Kind::F, "a.out", I, false),
    (9, "*.", Kind::F, ".bashrc", I, true),
    (10, "*.*", Kind::F, "Makefile", I, true),
    (11, "*.*", Kind::F, "a.tar.gz", I, true),
    (12, "abc.*", Kind::F, "abc", I, true),
    (12, "abc.*", Kind::F, "abcd", I, false),
    (13, "a?c.*", Kind::F, "a.c", I, false),
    (14, "[ab]*", Kind::F, "apple", I, true),
    (14, "[ab]*", Kind::F, "cherry", I, false),
    (15, "[a-c]?", Kind::F, "b1", I, true),
    (16, "[!a-c]*", Kind::F, "dog", I, true),
    (16, "[!a-c]*", Kind::F, "cat", I, false),
    (17, "[[]1].txt", Kind::F, "[1].txt", I, true),
    (18, "[]a]x", Kind::F, "]x", I, true),
    (19, "[a-]", Kind::F, "-", I, true),
    (20, "[A-Z]*", Kind::F, "zeta", I, true),
    (21, "src\\", Kind::D, "src", I, true),
    (21, "src\\", Kind::D, "lib/src", I, true),
    (22, "src\\*.c", Kind::F, "src/a.c", I, true),
    (22, "src\\*.c", Kind::F, "lib/src/a.c", I, false),
    (23, "src/*.c", Kind::F, "src/a.c", I, true),
    (24, ".\\*.c", Kind::F, "a.c", I, true),
    (24, ".\\*.c", Kind::F, "src/a.c", I, false),
    (25, "\\docs\\", Kind::D, "docs", I, true),
    (25, "\\docs\\", Kind::D, "a/docs", I, false),
    (26, "...\\test\\*.py", Kind::F, "test/a.py", I, true),
    (26, "...\\test\\*.py", Kind::F, "pkg/sub/test/a.py", I, true),
    (26, "...\\test\\*.py", Kind::F, "test/x/a.py", I, false),
    (27, "src\\...\\*.rs", Kind::F, "src/a.rs", I, true),
    (27, "src\\...\\*.rs", Kind::F, "src/x/y/a.rs", I, true),
    (27, "src\\...\\*.rs", Kind::F, "a.rs", I, false),
    (28, "caf\u{e9}.txt", Kind::F, "cafe\u{301}.txt", S, true),
    (29, "STRASSE", Kind::F, "stra\u{df}e", I, false),
    (30, "a*b*c", Kind::F, "aXbYc", S, true),
    (30, "a*b*c", Kind::F, "abc", S, true),
    (30, "a*b*c", Kind::F, "acb", S, false),
    (31, "*", Kind::F, "x", I, true),
    (31, "*", Kind::D, "x", I, false),
    (32, "*\\", Kind::D, "x", I, true),
    (37, "  *.h ", Kind::F, "a.h", I, true),
];

#[test]
fn normative_matching_rows() {
    for &(row, mask, kind, path, case, expected) in ROWS {
        assert_eq!(
            applies(mask, kind, path, case),
            expected,
            "row {row}: {mask:?} vs {kind:?} {path:?} ({case:?})"
        );
    }
}

#[test]
fn normative_error_rows() {
    let opts = MaskOptions { case: I };
    // Row 33.
    assert!(matches!(
        Mask::parse("[z-a]", opts),
        Err(MaskError::InvalidRange {
            at: 1,
            lo: 'z',
            hi: 'a',
            ..
        })
    ));
    // Row 34.
    assert_eq!(
        Mask::parse("a[b", opts).unwrap_err(),
        MaskError::UnclosedBracket {
            mask: "a[b".into(),
            at: 1
        }
    );
    // Row 35.
    assert_eq!(
        Mask::parse("a\\\\b", opts).unwrap_err(),
        MaskError::EmptyComponent {
            mask: "a\\\\b".into()
        }
    );
    // Row 36.
    assert_eq!(
        Mask::parse("x\\..\\y", opts).unwrap_err(),
        MaskError::DotComponent {
            mask: "x\\..\\y".into()
        }
    );
}

#[test]
fn row_37_items_are_trimmed() {
    let mask = Mask::parse("  *.h ", MaskOptions { case: I }).unwrap();
    assert_eq!(mask.source(), "*.h");
    assert!(mask.matches_name("a.h"));
}

#[test]
fn set_details() {
    let opts = MaskOptions { case: S };
    let m = |text: &str, name: &str| Mask::parse(text, opts).unwrap().matches_name(name);
    assert!(
        m("[^a]", "^"),
        "only '!' negates: [^a] is a set of '^' and 'a'"
    );
    assert!(m("[^a]", "a"));
    assert!(!m("[^a]", "b"));
    assert!(m("[-]x", "-x"), "[-] matches a literal '-' (D-07)");
    assert!(m("[!]]", "a"));
    assert!(!m("[!]]", "]"));
    assert!(m("[\\/]", "/"), "separators are literal inside sets");
    assert!(m("[a-c-e]", "-"));
    assert!(!m("[a-c-e]", "d"));
}

#[test]
fn mask_accessors() {
    let opts = MaskOptions { case: I };
    let m = Mask::parse("-obj\\", opts).unwrap();
    assert_eq!(
        (m.polarity(), m.target(), m.is_simple()),
        (Polarity::Exclude, MaskTarget::Folders, true)
    );
    let m = Mask::parse("src\\*.c", opts).unwrap();
    assert!(!m.is_simple());
    assert!(
        !m.matches_name("a.c"),
        "relative masks do not match bare names"
    );
    assert!(!m.matches_path(&RelPath::root()), "the root never matches");
}

#[test]
fn toolbar_parsing_example() {
    let opts = MaskOptions { case: S };
    let filter = NameFilter::parse("*.c;*.h;-*.bak;-obj\\;src\\", opts).unwrap();
    let lists = filter.to_lists();
    assert_eq!(lists.include_files, ["*.c", "*.h"]);
    assert_eq!(lists.exclude_files, ["*.bak"]);
    assert_eq!(lists.exclude_folders, ["obj\\"]);
    assert_eq!(lists.include_folders, ["src\\"]);
    assert_eq!(filter.to_filter_string(), "*.c;*.h;src\\;-*.bak;-obj\\");
    let again = NameFilter::parse(&filter.to_filter_string(), opts).unwrap();
    assert_eq!(again.to_lists(), lists, "INV-MASK-2");
}

#[test]
fn mask_list_round_trip_example() {
    let opts = MaskOptions { case: I };
    let list = MaskList::parse("*.c ;\t;-*.o;  build\\ ", opts).unwrap();
    assert_eq!(list.to_list_string(), "*.c;-*.o;build\\");
    let again = MaskList::parse(&list.to_list_string(), opts).unwrap();
    assert_eq!(again.to_list_string(), list.to_list_string());
}

#[test]
fn shadowing_containment_examples() {
    let opts = MaskOptions { case: I };
    let m = |text: &str| Mask::parse(text, opts).unwrap();
    assert!(m("*.c").is_covered_by(&m("*")), "(a)");
    assert!(m("*.c").is_covered_by(&m("*.*")), "(a)");
    assert!(m("*.C").is_covered_by(&m("*.c")), "(b)");
    assert!(m("test*.c").is_covered_by(&m("*.c")), "(c)");
    assert!(m("Makefile.am").is_covered_by(&m("Makefile*")), "(d)");
    assert!(m("Makefile").is_covered_by(&m("makefile")), "(e)");
    assert!(!m("*.h").is_covered_by(&m("*.c")));
    assert!(!m("*.c").is_covered_by(&m("test*.c")));
}

/// D-07: masks are compared in NFC, including set members and range bounds typed in NFD (a mask pasted from a
/// macOS file name) or as singleton decompositions.
#[test]
fn sets_are_compared_in_nfc() {
    let opts = MaskOptions { case: S };
    let m = |mask: &str, name: &str| Mask::parse(mask, opts).unwrap().matches_name(name);
    assert!(m("[é]x", "éx"));
    assert!(m("[e\u{301}]x", "éx"), "an NFD member is the NFC char");
    assert!(m("[e\u{301}]x", "e\u{301}x"));
    assert!(!m("[e\u{301}]x", "ex"));
    assert!(!m("[e\u{301}]x", "\u{301}x"));
    assert!(m("[!e\u{301}]", "e"));
    assert!(!m("[!e\u{301}]", "é"));
    assert!(m("[\u{212A}]*", "K1"), "KELVIN SIGN is K in NFC");
    assert!(m("[\u{212A}]*", "\u{212A}1"));
    assert!(m("[a-e\u{301}]", "z"), "the NFC range is a-é");
    let insensitive = MaskOptions { case: I };
    assert!(
        Mask::parse("[E\u{301}]*", insensitive)
            .unwrap()
            .matches_name("é.txt")
    );
    // The structure is the item's as typed; errors count the chars of the NFC text.
    assert_eq!(
        Mask::parse("e\u{301}[z-a]", opts).unwrap_err(),
        MaskError::InvalidRange {
            mask: "é[z-a]".into(),
            at: 2,
            lo: 'z',
            hi: 'a'
        }
    );
    let nfd = Mask::parse(" -e\u{301}\\[e\u{301}]* ", opts).unwrap();
    assert_eq!(
        nfd.source(),
        "-e\u{301}\\[e\u{301}]*",
        "the source stays as typed"
    );
    let nfc = Mask::parse("-é\\[é]*", opts).unwrap();
    assert!(nfd.is_covered_by(&nfc) && nfc.is_covered_by(&nfd));
}

/// §2.2.2 compares the NFC chars of a name with `simple_lower` (D-07), and tests a range through
/// `{c, simple_lower(c), simple_upper(c)}`. For the few names where case folding and these rules do not commute,
/// the result follows the NFC name (the reason P-MASK-3 is checked for regular names only).
#[test]
fn irregular_case_folds_follow_the_nfc_name() {
    let opts = MaskOptions { case: I };
    let m = |mask: &str, name: &str| Mask::parse(mask, opts).unwrap().matches_name(name);
    // `I` + COMBINING DOT ABOVE composes to `İ`, which maps to itself (its full lower case has two chars).
    assert!(!m("i*", "I\u{307}stanbul.txt"));
    assert!(!m("i*", "İstanbul.txt"));
    assert!(
        m("i*", "i\u{307}stanbul.txt"),
        "the fold of the NFD name is another name"
    );
    assert!(m("?stanbul.txt", "I\u{307}stanbul.txt"), "one char in NFC");
    // `W` + RING ABOVE has no precomposed form, `w` + RING ABOVE composes to `ẘ`.
    assert!(m("??", "W\u{30A}"));
    assert!(!m("??", "w\u{30A}"));
    assert!(m("?", "ẘ"));
    // A titlecase letter is neither lower case nor the upper case of its lower case.
    assert!(m("[ǅ-ǅ]", "ǅ"));
    assert!(!m("[ǅ-ǅ]", "ǆ"));
    assert!(m("[Ǆ-Ǆ]", "ǅ") && m("[Ǆ-Ǆ]", "ǆ"));
    assert!(
        m("ǅ", "ǆ") && m("ǅ", "Ǆ"),
        "literals compare simple lower cases"
    );
}
