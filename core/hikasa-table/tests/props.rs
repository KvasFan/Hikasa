//! AT.7: P-SELF, P-MIRROR, P-KEYORDER.

mod common;

use std::sync::Arc;

use common::cmp;
use hikasa_core::ContentKind;
use hikasa_table::{
    FirstLine, Importance, Regional, RowKind, Sheet, SheetRules, SourceKind, TableCompareOptions,
    Workbook,
};
use proptest::prelude::*;

fn table(rows: &[Vec<String>]) -> Arc<Workbook> {
    let mut all = vec![vec!["k".to_owned(), "a".to_owned(), "b".to_owned()]];
    all.extend(rows.iter().cloned());
    Arc::new(Workbook::from_sheets(
        vec![Sheet::from_text_rows(
            "S",
            &all,
            FirstLine::Names,
            Regional::default(),
        )],
        SourceKind::Delimited,
    ))
}

fn value() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        (0u8..6).prop_map(|n| n.to_string()),
        (0u8..4).prop_map(|n| format!("{n}.0")),
        prop::sample::select(vec!["x", "X", "y", " x", "2024-01-01"]).prop_map(str::to_owned),
    ]
}

fn rows(n: usize) -> impl Strategy<Value = Vec<Vec<String>>> {
    prop::collection::vec(prop::collection::vec(value(), 3), 0..n)
}

fn keyed() -> TableCompareOptions {
    TableCompareOptions {
        default_sheet: SheetRules {
            keys: vec![hikasa_table::ColumnRef::Position(1)],
            ..SheetRules::default()
        },
        ..TableCompareOptions::default()
    }
}

/// Row states as (kind, importance, left row, right row), sides swapped when `swap`.
fn states(
    c: &hikasa_table::TableComparison,
    swap: bool,
) -> Vec<(RowKind, Importance, Option<u32>, Option<u32>)> {
    c.sheets()[0]
        .rows()
        .iter()
        .map(|r| {
            if swap {
                let k = match r.kind() {
                    RowKind::LeftOnly => RowKind::RightOnly,
                    RowKind::RightOnly => RowKind::LeftOnly,
                    k => k,
                };
                (k, r.importance(), r.right(), r.left())
            } else {
                (r.kind(), r.importance(), r.left(), r.right())
            }
        })
        .collect()
}

/// A row's kind, importance and the two rows' values.
type Pairing = (
    RowKind,
    Importance,
    Option<Vec<String>>,
    Option<Vec<String>>,
);

proptest! {
    #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(256), ..ProptestConfig::default() })]

    /// P-SELF: a table compared with itself is all same, keyed and unkeyed.
    #[test]
    fn p_self(r in rows(30), use_keys in any::<bool>()) {
        let t = table(&r);
        let opts = if use_keys { keyed() } else { TableCompareOptions::default() };
        let c = cmp(&t, &t, &opts);
        prop_assert_eq!(c.kind(false, false), ContentKind::RulesSame);
        prop_assert!(c.sheets()[0].rows().iter().all(|x| x.kind() == RowKind::Same));
    }

    /// P-MIRROR: swapping the sides mirrors every row and the statistics.
    #[test]
    fn p_mirror(a in rows(20), b in rows(20), use_keys in any::<bool>()) {
        let (l, r) = (table(&a), table(&b));
        let opts = if use_keys { keyed() } else { TableCompareOptions::default() };
        let (x, y) = (cmp(&l, &r, &opts), cmp(&r, &l, &opts));
        prop_assert_eq!(x.kind(false, false), y.kind(false, false));
        let (sx, sy) = (x.sheets()[0].stats(), y.sheets()[0].stats());
        prop_assert_eq!((sx.left_rows, sx.right_rows), (sy.right_rows, sy.left_rows));
        if use_keys {
            // Keyed output is in key order on both runs: the rows and the statistics mirror one to one.
            prop_assert_eq!(sx.cells_important, sy.cells_important);
            prop_assert_eq!(
                sx.left_only_important + sx.left_only_unimportant,
                sy.right_only_important + sy.right_only_unimportant
            );
            prop_assert_eq!(states(&x, false), states(&y, true));
        } else {
            // The engines prefer the left side in gaps, so an unkeyed alignment mirrors only up to the pairing
            // of a gap: every row of each side still appears exactly once.
            for c in [&x, &y] {
                let rows = c.sheets()[0].rows();
                let mut left: Vec<u32> = rows.iter().filter_map(|r| r.left()).collect();
                let mut right: Vec<u32> = rows.iter().filter_map(|r| r.right()).collect();
                left.sort_unstable();
                right.sort_unstable();
                let st = c.sheets()[0].stats();
                prop_assert_eq!(left, (0..st.left_rows as u32).collect::<Vec<_>>());
                prop_assert_eq!(right, (0..st.right_rows as u32).collect::<Vec<_>>());
            }
        }
    }

    /// P-KEYORDER: a keyed join does not depend on the order of the input rows.
    #[test]
    fn p_keyorder(a in rows(25), b in rows(25), seed in any::<u64>()) {
        let shuffle = |v: &[Vec<String>], seed: u64| {
            let mut v = v.to_vec();
            let mut rng = common::Rng(seed | 1);
            for i in (1..v.len()).rev() {
                v.swap(i, rng.below(i as u64 + 1) as usize);
            }
            v
        };
        let (l, r) = (table(&a), table(&b));
        let (ls, rs) = (table(&shuffle(&a, seed)), table(&shuffle(&b, seed ^ 0x9E37)));
        let (x, y) = (cmp(&l, &r, &keyed()), cmp(&ls, &rs, &keyed()));
        prop_assert_eq!(x.kind(false, false), y.kind(false, false));
        let multiset = |c: &hikasa_table::TableComparison, lt: &[Vec<String>], rt: &[Vec<String>]| {
            let mut v: Vec<Pairing> = c.sheets()[0]
                .rows()
                .iter()
                .map(|r| (r.kind(), r.importance(), r.left().map(|i| lt[i as usize].clone()), r.right().map(|i| rt[i as usize].clone())))
                .collect();
            v.sort_by(|p, q| format!("{p:?}").cmp(&format!("{q:?}")));
            v
        };
        // Keys compare typed: `3` and `3.0` are one key.
        let unique = |v: &[Vec<String>]| {
            let norm = |s: &String| Regional::default().parse_any(s).map_or_else(|| s.clone(), |p| format!("{p:?}"));
            let mut k: Vec<String> = v.iter().map(|r| norm(&r[0])).collect();
            k.sort();
            k.windows(2).all(|w| w[0] != w[1])
        };
        if unique(&a) && unique(&b) {
            prop_assert_eq!(multiset(&x, &a, &b), multiset(&y, &shuffle(&a, seed), &shuffle(&b, seed ^ 0x9E37)));
        } else {
            let kinds = |c: &hikasa_table::TableComparison| {
                let mut k: Vec<(RowKind, Importance)> = c.sheets()[0].rows().iter().map(|r| (r.kind(), r.importance())).collect();
                k.sort_by_key(|t| format!("{t:?}"));
                k
            };
            prop_assert_eq!(kinds(&x).len(), kinds(&y).len());
        }
    }
}
