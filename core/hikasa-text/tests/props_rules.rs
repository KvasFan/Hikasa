//! INV-RULES-1: `TextRules::from_toml(to_toml(r)) == r` for every value (TOML round trip of every field), and
//! the `--unimportant` vocabulary round trip.

mod common;

use common::config;
use hikasa_text::{AlgorithmChoice, TextPattern, TextRules, UnimportantItem};
use proptest::prelude::*;

fn any_text() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9 _.$\"'\\\\\t\n#=\\[\\]{}é€\u{1F600}-]{1,12}").unwrap()
}

fn element_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Za-z][A-Za-z0-9 _\"'é-]{0,10}[A-Za-z0-9]").unwrap()
}

fn rules() -> impl Strategy<Value = TextRules> {
    (
        0u32..=1,
        prop::collection::btree_map(element_name(), any::<bool>(), 0..4),
        prop::array::uniform8(any::<bool>()),
        prop::collection::vec((any_text(), any::<bool>(), any::<bool>()), 0..4),
        0..AlgorithmChoice::ALL.len(),
        any::<u32>(),
        any::<u16>(),
        prop::array::uniform2(any::<bool>()),
    )
        .prop_map(
            |(schema, elements, i, patterns, algorithm, skew, threshold, a)| {
                let mut r = TextRules {
                    schema_version: schema,
                    ..TextRules::default()
                };
                r.importance.elements = elements;
                r.importance.leading_whitespace = i[0];
                r.importance.embedded_whitespace = i[1];
                r.importance.trailing_whitespace = i[2];
                r.importance.everything_else = i[3];
                r.importance.everything_else_case_sensitive = i[4];
                r.importance.orphans_always_important = i[5];
                r.importance.compare_line_endings = i[6];
                r.alignment.closeness = i[7];
                r.importance.unimportant = patterns
                    .into_iter()
                    .map(|(pattern, regex, match_case)| TextPattern {
                        pattern,
                        regex,
                        match_case,
                    })
                    .collect();
                r.alignment.algorithm = AlgorithmChoice::ALL[algorithm];
                r.alignment.skew_tolerance = skew;
                r.alignment.closeness_threshold_permille = threshold;
                r.alignment.never_align_differences = a[0];
                r
            },
        )
}

fn items() -> impl Strategy<Value = Vec<UnimportantItem>> {
    prop::collection::vec(
        prop_oneof![
            Just(UnimportantItem::LeadingWhitespace),
            Just(UnimportantItem::EmbeddedWhitespace),
            Just(UnimportantItem::TrailingWhitespace),
            Just(UnimportantItem::Whitespace),
            Just(UnimportantItem::Case),
            Just(UnimportantItem::EverythingElse),
            element_name().prop_map(UnimportantItem::Element),
            Just(UnimportantItem::Element("case".into())),
            Just(UnimportantItem::Element("Whitespace".into())),
        ],
        0..6,
    )
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_rules.txt"))]

    /// INV-RULES-1.
    #[test]
    fn inv_rules_1_toml_round_trip(r in rules()) {
        let text = r.to_toml();
        let (back, warnings) = TextRules::from_toml(&text).unwrap();
        prop_assert!(warnings.is_empty(), "{:?}", warnings);
        prop_assert_eq!(back, r);
    }

    /// The vocabulary: `parse_list` of the joined canonical words gives the items back.
    #[test]
    fn unimportant_vocabulary_round_trip(items in items()) {
        let list: Vec<String> = items.iter().map(|i| i.as_str().into_owned()).collect();
        let parsed = UnimportantItem::parse_list(&list.join(", ")).unwrap();
        prop_assert_eq!(parsed, items);
    }
}
