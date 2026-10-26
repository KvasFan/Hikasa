//! P-SHADOW-1 (§2.12.3, §6.2.4): shadowing is sound. Whenever the shadow report says that an entry is shadowed,
//! no file name is ever detected as that entry, and every name its mask matches is matched by a covering entry.

mod common;

use std::sync::Arc;

use common::config;
use hikasa_formats::{
    DetectReason, EnabledOverrides, FileFormat, FormatId, FormatKind, FormatRegistry, Origin,
    RegexLimits, RegistryEntry, ShadowStatus,
};
use proptest::prelude::*;

fn mask_item() -> impl Strategy<Value = String> {
    let simple = prop::collection::vec(
        prop::sample::select(vec!["a", "b", ".", "*", "?", "c", "[ab]", "[!a]", "x"]),
        1..5,
    )
    .prop_map(|v| v.concat());
    prop_oneof![
        4 => simple.clone(),
        1 => simple.prop_map(|s| format!("-{s}")),
        1 => prop::sample::select(vec!["*", "*.*", "*.c", "a*", "*.", "*c"]).prop_map(str::to_owned),
    ]
}

fn mask() -> impl Strategy<Value = String> {
    prop::collection::vec(mask_item(), 1..4).prop_map(|v| v.join(";"))
}

fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['a', 'b', 'c', 'x', '.', 'A', 'B', '-']),
        0..7,
    )
    .prop_map(|c| c.into_iter().collect())
}

fn registry(masks: &[String]) -> Option<FormatRegistry> {
    let entries = masks
        .iter()
        .enumerate()
        .map(|(k, m)| {
            let mut f = FileFormat::new(
                FormatId::new(format!("f{k}")).unwrap(),
                format!("F{k}"),
                FormatKind::Text,
            );
            f.mask = m.clone();
            RegistryEntry::new(Arc::new(f), true, Origin::BuiltIn)
        })
        .collect();
    FormatRegistry::from_entries(entries, RegexLimits::default()).ok()
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_shadow.txt"))]

    /// P-SHADOW-1.
    #[test]
    fn p_shadow_1_soundness(masks in prop::collection::vec(mask(), 2..5), names in prop::collection::vec(name(), 1..40)) {
        let Some(r) = registry(&masks) else {
            return Ok(());
        };
        let none = EnabledOverrides::default();
        let report = r.shadow_report(&none);
        for (id, status) in &report {
            let ShadowStatus::Shadowed { by } = status else {
                continue;
            };
            prop_assert!(!by.is_empty());
            let me = r.get(id).unwrap();
            for n in &names {
                // The shadowed entry is never the detected one.
                if let DetectReason::Mask { id: got, .. } = r.detect(n, &none).unwrap().reason {
                    prop_assert_ne!(&got, id, "{:?} detected as shadowed {}", n, id);
                }
                // Every name an include item of the shadowed entry matches is matched by a covering entry.
                let matched = me.mask().unwrap().masks().iter().any(|m| {
                    m.polarity() == hikasa_core::mask::Polarity::Include && m.matches_name(n)
                });
                if matched {
                    let covered = by.iter().any(|b| {
                        r.get(b).unwrap().mask().unwrap().masks().iter().any(|m| m.matches_name(n))
                    });
                    prop_assert!(covered, "{:?}: {} not covered by {:?}", n, id, by);
                }
            }
        }
    }
}

#[test]
fn shadow_report_examples() {
    let r = registry(&["*.c".into(), "test*.c;-x".into(), "test*.c".into()]).unwrap();
    let report = r.shadow_report(&EnabledOverrides::default());
    assert_eq!(report[0].1, ShadowStatus::Visible);
    assert_eq!(
        report[1].1,
        ShadowStatus::Shadowed {
            by: vec![FormatId::new("f0").unwrap()]
        }
    );
    // f1 has an exclude item, so it never shadows; f0 still does.
    assert_eq!(
        report[2].1,
        ShadowStatus::Shadowed {
            by: vec![FormatId::new("f0").unwrap()]
        }
    );
}

/// P-SHADOW-1 is not vacuous: random registries regularly contain shadowed entries.
#[test]
fn p_shadow_1_reaches_shadowed_entries() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let strategy = prop::collection::vec(mask(), 2..5);
    let mut shadowed = 0;
    for _ in 0..256 {
        let masks = strategy.new_tree(&mut runner).unwrap().current();
        if let Some(r) = registry(&masks)
            && r.shadow_report(&EnabledOverrides::default())
                .iter()
                .any(|(_, s)| matches!(s, ShadowStatus::Shadowed { .. }))
        {
            shadowed += 1;
        }
    }
    assert!(
        shadowed >= 30,
        "{shadowed} of 256 registries have a shadowed entry"
    );
}
