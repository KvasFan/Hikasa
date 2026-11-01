//! P-RULES-2 (INV-RULES-2): `FolderRules` round-trips through TOML for generated values of every field, and the
//! timestamp properties of §2.4 (symmetry, reflexivity, tolerance monotonicity, DST ⊂ TZ).

mod common;

use common::config;
use hikasa_core::{ContentMethod, FileTime, NameFilterLists, Side};
use hikasa_folder::{
    ArchiveHandling, CaseMode, Change, Cmp, ContentRule, Criteria, DateCutoff, DateFilter,
    FilterRules, FolderRules, FolderRulesPatch, Handling, NameAlignment, OtherFilters, SizeFilter,
    TimeRelation, TimestampRule, TzOffset, UnixCriteria, compare_times,
};
use hikasa_vfs::DosAttrs;
use proptest::prelude::*;

/// Valid mask items (the lists are validated when parsed).
const MASKS: [&str; 10] = [
    "*.c",
    "obj\\",
    "src/",
    "[ ]x",
    "...\\*.rs",
    ".\\top\\",
    "a?b",
    "[!a-c]*",
    "*.",
    "p\\f",
];

fn lists() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(
        prop::sample::select(&MASKS[..]).prop_map(str::to_owned),
        0..3,
    )
}

fn file_time() -> impl Strategy<Value = FileTime> {
    (any::<i64>(), 0u32..1_000_000_000).prop_map(|(s, n)| FileTime::from_unix(s, n).unwrap())
}

fn side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Left), Just(Side::Right)]
}

fn cmp() -> impl Strategy<Value = Cmp> {
    prop_oneof![Just(Cmp::Less), Just(Cmp::Greater)]
}

fn method() -> impl Strategy<Value = ContentMethod> {
    prop_oneof![
        Just(ContentMethod::None),
        Just(ContentMethod::Crc),
        Just(ContentMethod::Binary),
        Just(ContentMethod::Rules)
    ]
}

fn rules() -> impl Strategy<Value = FolderRules> {
    let timestamps = (
        any::<bool>(),
        any::<u32>(),
        any::<bool>(),
        any::<bool>(),
        proptest::option::of((side(), -12i8..=12)),
    )
        .prop_map(
            |(enabled, tolerance_secs, ignore_dst, ignore_timezone, offset)| TimestampRule {
                enabled,
                tolerance_secs,
                ignore_dst,
                ignore_timezone,
                offset: offset.map(|(side, hours)| TzOffset { side, hours }),
            },
        );
    let criteria = (
        any::<bool>(),
        timestamps,
        any::<bool>(),
        any::<u32>(),
        (any::<bool>(), any::<bool>(), any::<bool>()),
        (method(), any::<bool>(), any::<bool>()),
    )
        .prop_map(
            |(size, timestamps, filename_case, bits, (p, o, g), (m, skip, ovr))| Criteria {
                size,
                timestamps,
                filename_case,
                dos_attributes: DosAttrs::from_bits_truncate(bits),
                unix: UnixCriteria {
                    permissions: p,
                    owner: o,
                    group: g,
                },
                content: ContentRule {
                    method: m,
                    skip_if_quick_same: skip,
                    override_quick: ovr,
                },
            },
        );
    let alignment = (
        prop_oneof![
            Just(CaseMode::Auto),
            Just(CaseMode::Sensitive),
            Just(CaseMode::Insensitive)
        ],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(case, align_extensions, align_normalization)| NameAlignment {
                case,
                align_extensions,
                align_normalization,
            },
        );
    let cutoff = prop_oneof![
        file_time().prop_map(|at| DateCutoff::Absolute { at }),
        any::<u32>().prop_map(|days| DateCutoff::DaysAgo { days }),
    ];
    let filters = (
        (lists(), lists(), lists(), lists()),
        any::<bool>(),
        proptest::option::of((cmp(), any::<u64>())),
        proptest::option::of((cmp(), cutoff)),
    )
        .prop_map(
            |((inf, exf, ind, exd), exclude_protected, size, date)| FilterRules {
                names: NameFilterLists {
                    include_files: inf,
                    exclude_files: exf,
                    include_folders: ind,
                    exclude_folders: exd,
                },
                other: OtherFilters {
                    exclude_protected,
                    size: size.map(|(cmp, bytes)| SizeFilter { cmp, bytes }),
                    date: date.map(|(cmp, cutoff)| DateFilter { cmp, cutoff }),
                },
            },
        );
    let handling = (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(follow_symlinks, flatten, scan_orphan_folders)| Handling {
            follow_symlinks,
            flatten,
            scan_orphan_folders,
            archives: ArchiveHandling::default(),
        },
    );
    (0u32..=1, criteria, alignment, filters, handling).prop_map(
        |(schema_version, criteria, alignment, filters, handling)| FolderRules {
            schema_version,
            criteria,
            alignment,
            filters,
            handling,
            open_archives: Vec::new(),
        },
    )
}

fn rule() -> impl Strategy<Value = TimestampRule> {
    (
        0u32..10_000,
        any::<bool>(),
        any::<bool>(),
        proptest::option::of((side(), -12i8..=12)),
    )
        .prop_map(
            |(tolerance_secs, ignore_dst, ignore_timezone, offset)| TimestampRule {
                enabled: true,
                tolerance_secs,
                ignore_dst,
                ignore_timezone,
                offset: offset.map(|(side, hours)| TzOffset { side, hours }),
            },
        )
}

/// Times near each other (so that hour multiples and tolerances matter) and far apart.
fn times() -> impl Strategy<Value = (FileTime, FileTime)> {
    let base = -10_000_000_000i64..10_000_000_000;
    prop_oneof![
        (
            base.clone(),
            -200_000i64..200_000,
            0u32..1_000_000_000,
            0u32..1_000_000_000
        )
            .prop_map(|(b, d, n1, n2)| (
                FileTime::from_unix(b, n1).unwrap(),
                FileTime::from_unix(b + d, n2).unwrap()
            )),
        (base, 0i64..30, -3i64..3, 0u32..1_000_000_000).prop_map(|(b, k, s, n)| (
            FileTime::from_unix(b, 0).unwrap(),
            FileTime::from_unix(b + k * 3600 + s, n).unwrap()
        )),
        (file_time(), file_time()),
    ]
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_rules.txt"))]

    /// P-RULES-2 / INV-RULES-2.
    #[test]
    fn p_rules_2_toml_round_trip(rules in rules()) {
        let text = rules.to_toml();
        let (back, warnings) = FolderRules::from_toml(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
        prop_assert!(warnings.is_empty(), "{:?}", warnings);
        prop_assert_eq!(&back, &rules, "{}", text);
        prop_assert_eq!(back.to_toml(), text);
        // JSON transport (IPC) round-trips too, and so does a patch carrying every optional rule.
        let json = serde_json::to_string(&rules).unwrap();
        prop_assert_eq!(serde_json::from_str::<FolderRules>(&json).unwrap(), rules.clone());
        let patch = FolderRulesPatch {
            tz_offset: Some(rules.criteria.timestamps.offset.map_or(Change::Clear, Change::Set)),
            size_filter: Some(rules.filters.other.size.map_or(Change::Clear, Change::Set)),
            date_filter: Some(rules.filters.other.date.map_or(Change::Clear, Change::Set)),
            ..FolderRulesPatch::default()
        };
        let json = serde_json::to_string(&patch).unwrap();
        prop_assert_eq!(serde_json::from_str::<FolderRulesPatch>(&json).unwrap(), patch.clone());
        let mut applied = FolderRules::default();
        applied.apply(&patch).unwrap();
        prop_assert_eq!(applied.criteria.timestamps.offset, rules.criteria.timestamps.offset);
        prop_assert_eq!(applied.filters.other.size, rules.filters.other.size);
        prop_assert_eq!(applied.filters.other.date, rules.filters.other.date);
    }

    /// §2.4 symmetry: `compare(l, r) = mirror(compare(r, l))` (an offset moves with its side).
    #[test]
    fn p_time_symmetry((l, r) in times(), rule in rule()) {
        let mut swapped = rule;
        if let Some(offset) = &mut swapped.offset {
            offset.side = offset.side.opposite();
        }
        prop_assert_eq!(compare_times(l, r, &rule), compare_times(r, l, &swapped).mirrored());
    }

    /// §2.4 reflexivity: `compare(t, t) = Same` (without an offset).
    #[test]
    fn p_time_reflexive(t in file_time(), mut rule in rule()) {
        rule.offset = None;
        prop_assert_eq!(compare_times(t, t, &rule), TimeRelation::Same);
    }

    /// §2.4 tolerance monotonicity and DST ⊂ TZ.
    #[test]
    fn p_time_monotone((l, r) in times(), rule in rule(), extra in 0u32..10_000) {
        let wider = TimestampRule { tolerance_secs: rule.tolerance_secs + extra, ..rule };
        if compare_times(l, r, &rule) == TimeRelation::Same {
            prop_assert_eq!(compare_times(l, r, &wider), TimeRelation::Same);
        }
        let dst = TimestampRule { ignore_dst: true, ignore_timezone: false, ..rule };
        let tz = TimestampRule { ignore_dst: false, ignore_timezone: true, ..rule };
        if compare_times(l, r, &dst) == TimeRelation::Same {
            prop_assert_eq!(compare_times(l, r, &tz), TimeRelation::Same);
        }
    }
}
