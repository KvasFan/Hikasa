//! Registry unit tests: detection (first enabled match, excludes, D-10), forced formats, shadowing (§2.12.3),
//! session overrides (FF-002), and the merge steps of registry loading (§2.12.4).

use super::*;
use crate::model::FormatKind;

fn fmt(id: &str, mask: &str) -> Arc<FileFormat> {
    let mut f = FileFormat::new(
        FormatId::new(id).unwrap(),
        id.to_uppercase(),
        FormatKind::Text,
    );
    f.mask = mask.to_owned();
    Arc::new(f)
}

fn registry(list: &[(&str, &str, bool)]) -> FormatRegistry {
    FormatRegistry::from_entries(
        list.iter()
            .map(|(id, mask, enabled)| RegistryEntry::new(fmt(id, mask), *enabled, Origin::BuiltIn))
            .collect(),
        RegexLimits::default(),
    )
    .unwrap()
}

fn id(s: &str) -> FormatId {
    FormatId::new(s).unwrap()
}

fn detected(r: &FormatRegistry, name: &str, overrides: &EnabledOverrides) -> DetectReason {
    r.detect(name, overrides).unwrap().reason
}

#[test]
fn detection_table() {
    let r = registry(&[
        ("c", "*.c;*.h;-test_*", true),
        ("cpp", "*.cpp;*.C;*.h", true),
        ("off", "*.txt", false),
        ("all", "*.*", true),
    ]);
    let none = EnabledOverrides::default();
    let mask = |id_: &str, m: &str| DetectReason::Mask {
        id: id(id_),
        mask: m.to_owned(),
    };
    assert_eq!(detected(&r, "x.c", &none), mask("c", "*.c"));
    // Case-insensitive always (D-10): `main.C` is still detected, by the first matching include item.
    assert_eq!(detected(&r, "MAIN.C", &none), mask("c", "*.c"));
    assert_eq!(detected(&r, "y.h", &none), mask("c", "*.h"));
    // An exclude item skips the entry; the next entry may match.
    assert_eq!(detected(&r, "test_a.h", &none), mask("cpp", "*.h"));
    assert_eq!(detected(&r, "test_a.c", &none), mask("cpp", "*.C"));
    // Disabled entries are skipped; overrides enable them for one session (FF-002).
    assert_eq!(detected(&r, "a.txt", &none), mask("all", "*.*"));
    let on = EnabledOverrides([(id("off"), true)].into_iter().collect());
    assert_eq!(detected(&r, "a.txt", &on), mask("off", "*.txt"));
    let c_off = EnabledOverrides([(id("c"), false)].into_iter().collect());
    assert_eq!(detected(&r, "x.c", &c_off), mask("cpp", "*.C"));
    // Only the name part is matched.
    assert_eq!(detected(&r, "src/dir.c/x.cpp", &none), mask("cpp", "*.cpp"));
    assert_eq!(detected(&r, r"C:\dir\x.cpp", &none), mask("cpp", "*.cpp"));
    // `*.*` also matches names without an extension (D-07); a registry without it falls back.
    assert_eq!(detected(&r, "Makefile", &none), mask("all", "*.*"));
    let small = registry(&[("c", "*.c", true)]);
    let d = small.detect("README", &none).unwrap();
    assert_eq!(d.reason, DetectReason::Fallback);
    assert_eq!(d.format.name(), "Everything Else");
    assert!(Arc::ptr_eq(&d.format, small.everything_else()));
}

#[test]
fn compiled_formats_are_cached_and_errors_returned() {
    let mut broken = FileFormat::new(id("broken"), "Broken", FormatKind::Text);
    broken.mask = "*.b".into();
    broken.text.tab_size = 0;
    let r = FormatRegistry::from_entries(
        vec![
            RegistryEntry::new(fmt("ok", "*.ok"), true, Origin::BuiltIn),
            RegistryEntry::new(Arc::new(broken), true, Origin::BuiltIn),
        ],
        RegexLimits::default(),
    )
    .unwrap();
    let a = r.compiled(&id("ok")).unwrap();
    let b = r.compiled(&id("ok")).unwrap();
    assert!(Arc::ptr_eq(&a, &b), "compiled once per entry");
    let none = EnabledOverrides::default();
    assert!(matches!(
        r.detect("x.b", &none),
        Err(FormatError::Invalid { .. })
    ));
    assert!(matches!(
        r.compiled(&id("nope")),
        Err(FormatError::UnknownFormat { .. })
    ));
}

#[test]
fn find_and_force() {
    let r = registry(&[("c-cpp", "*.c", true), ("rust", "*.rs", false)]);
    assert_eq!(r.find("rust").unwrap().id().as_str(), "rust");
    assert_eq!(r.find(" C-CPP ").unwrap().id().as_str(), "c-cpp");
    assert!(r.find("C-cpp-x").is_none());
    let d = r.force("Rust").unwrap();
    assert_eq!(d.reason, DetectReason::Forced { id: id("rust") });
    assert!(matches!(
        r.force("cobol"),
        Err(FormatError::UnknownFormat { name }) if name == "cobol"
    ));
}

#[test]
fn from_entries_validates() {
    let dup = FormatRegistry::from_entries(
        vec![
            RegistryEntry::new(fmt("a", "*.a"), true, Origin::BuiltIn),
            RegistryEntry::new(fmt("a", "*.b"), true, Origin::BuiltIn),
        ],
        RegexLimits::default(),
    );
    assert!(matches!(dup, Err(FormatError::DuplicateId { id }) if id == "a"));
    let sep = FormatRegistry::from_entries(
        vec![RegistryEntry::new(
            fmt("a", "src/*.a"),
            true,
            Origin::BuiltIn,
        )],
        RegexLimits::default(),
    );
    assert!(matches!(sep, Err(FormatError::Invalid { .. })));
    let folder = FormatRegistry::from_entries(
        vec![RegistryEntry::new(fmt("a", "obj\\"), true, Origin::BuiltIn)],
        RegexLimits::default(),
    );
    assert!(matches!(folder, Err(FormatError::Invalid { .. })));
    let syntax = FormatRegistry::from_entries(
        vec![RegistryEntry::new(fmt("a", "[x"), true, Origin::BuiltIn)],
        RegexLimits::default(),
    );
    assert!(matches!(syntax, Err(FormatError::Mask { .. })));
    let entry = RegistryEntry::new(fmt("a", "*.a"), true, Origin::BuiltIn);
    assert!(entry.mask().is_none());
}

#[test]
fn shadowing_rules() {
    let r = registry(&[
        ("c", "*.c;*.h", true),
        ("star-c", "test*.c;x.c;*.h", true),
        ("pre", "make*", true),
        ("mk", "makefile;make*.am", true),
        ("excl", "*.x;-a.x", true),
        ("by-excl", "b.x", true),
        ("partial", "*.c;*.y", true),
        ("manual", "", true),
        ("only-excl", "-*.z", true),
        ("disabled", "*.c", false),
        ("all", "*", true),
        ("late", "*.q", true),
    ]);
    let report = r.shadow_report(&EnabledOverrides::default());
    let status = |name: &str| {
        report
            .iter()
            .find(|(i, _)| i.as_str() == name)
            .map(|(_, s)| s.clone())
            .unwrap()
    };
    let shadowed = |ids: &[&str]| ShadowStatus::Shadowed {
        by: ids.iter().map(|i| id(i)).collect(),
    };
    assert_eq!(status("c"), ShadowStatus::Visible);
    // (c) `*.c` covers `test*.c`; (b)/(e) identical items.
    assert_eq!(status("star-c"), shadowed(&["c"]));
    // (d) `make*` covers `makefile` and `make*.am`.
    assert_eq!(status("mk"), shadowed(&["pre"]));
    // An earlier entry with exclude items never shadows.
    assert_eq!(status("by-excl"), ShadowStatus::Visible);
    assert_eq!(status("partial"), ShadowStatus::Visible);
    assert_eq!(status("manual"), ShadowStatus::ManualOnly);
    assert_eq!(status("only-excl"), ShadowStatus::ManualOnly);
    assert_eq!(status("disabled"), ShadowStatus::Disabled);
    assert_eq!(status("all"), ShadowStatus::Visible);
    // (a) `*` covers everything after it.
    assert_eq!(status("late"), shadowed(&["all"]));
    // With "all" disabled for the session, "late" becomes visible.
    let off = EnabledOverrides([(id("all"), false)].into_iter().collect());
    let report = r.shadow_report(&off);
    assert!(report.contains(&(id("late"), ShadowStatus::Visible)));
    assert!(report.contains(&(id("all"), ShadowStatus::Disabled)));
    assert_eq!(
        serde_json::to_string(&shadowed(&["c"])).unwrap(),
        r#"{"status":"shadowed","by":["c"]}"#
    );
}

#[test]
fn shadowed_by_lists_entries_in_registry_order() {
    let r = registry(&[
        ("h", "*.h", true),
        ("c", "*.c", true),
        ("both", "*.c;*.h", true),
    ]);
    let report = r.shadow_report(&EnabledOverrides::default());
    assert_eq!(
        report[2].1,
        ShadowStatus::Shadowed {
            by: vec![id("h"), id("c")]
        }
    );
}

#[test]
fn user_formats_replace_built_ins_in_place() {
    let r = registry(&[("a", "*.a", true), ("b", "*.b", false), ("c", "*.c", true)]);
    let (r, warnings) = r.with_user_formats(vec![
        (fmt("b", "*.bb"), PathBuf::from("/cfg/formats/b.toml")),
        (fmt("zz", "*.z"), PathBuf::from("/cfg/formats/zz.toml")),
        (fmt("m", "*.m"), PathBuf::from("/cfg/formats/m.toml")),
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let ids: Vec<_> = r.entries().iter().map(|e| e.id().as_str()).collect();
    assert_eq!(ids, ["a", "b", "c", "m", "zz"]);
    let b = r.get(&id("b")).unwrap();
    assert_eq!(b.format.mask, "*.bb");
    assert!(b.mask().is_some());
    assert!(!b.enabled, "the built-in's flag is kept");
    assert_eq!(
        b.origin,
        Origin::User {
            path: PathBuf::from("/cfg/formats/b.toml")
        }
    );
    assert!(r.get(&id("m")).unwrap().enabled);
    assert_eq!(
        r.detect("x.bb", &EnabledOverrides::default())
            .unwrap()
            .reason,
        DetectReason::Fallback,
        "b is disabled"
    );
    assert_eq!(
        r.detect("x.m", &EnabledOverrides::default())
            .unwrap()
            .reason,
        DetectReason::Mask {
            id: id("m"),
            mask: "*.m".into()
        }
    );
}

/// One broken user file never costs the others or the built-ins (§2.12.4: skipped with `user_format_skipped`,
/// the CLI stays usable; wave-2 review).
#[test]
fn broken_user_formats_are_skipped_with_warnings() {
    let r = registry(&[("a", "*.a", true), ("b", "*.b", true)]);
    let (r, warnings) = r.with_user_formats(vec![
        (
            fmt("mine", "src/*.c"),
            PathBuf::from("/cfg/formats/mine.toml"),
        ),
        (fmt("bad", "a[b"), PathBuf::from("/cfg/formats/bad.toml")),
        (
            fmt("b", "*.c;sub\\*.h"),
            PathBuf::from("/cfg/formats/b.toml"),
        ),
        (
            fmt("q", "*.q"),
            PathBuf::from("/cfg/formats/other-name.toml"),
        ),
        (fmt("ok", "*.ok"), PathBuf::from("/cfg/formats/ok.toml")),
        (fmt("ok", "*.ok2"), PathBuf::from("/elsewhere/ok.toml")),
    ]);
    let ids: Vec<_> = r.entries().iter().map(|e| e.id().as_str()).collect();
    assert_eq!(ids, ["a", "b", "ok"]);
    assert_eq!(r.get(&id("b")).unwrap().origin, Origin::BuiltIn);
    assert_eq!(r.get(&id("ok")).unwrap().format.mask, "*.ok");
    let messages: Vec<_> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages.len(), 5, "{messages:#?}");
    assert!(
        warnings
            .iter()
            .all(|w| w.code == WarningCode::UserFormatSkipped)
    );
    assert!(messages[0].starts_with("/cfg/formats/mine.toml: skipped: format mine: mask:"));
    assert!(messages[0].contains("separator"), "{}", messages[0]);
    assert!(messages[1].starts_with("/cfg/formats/bad.toml: skipped: format bad: mask:"));
    assert!(messages[2].starts_with("/cfg/formats/b.toml: skipped:"));
    assert_eq!(
        messages[3],
        "/cfg/formats/other-name.toml: skipped: the format id `q` is not the file name `other-name.toml`"
    );
    assert_eq!(
        messages[4],
        "/elsewhere/ok.toml: skipped: another user format already has the id `ok`"
    );
    // The surviving registry works.
    assert!(r.detect("x.a", &EnabledOverrides::default()).is_ok());
    assert_eq!(r.shadow_report(&EnabledOverrides::default()).len(), 3);
}

#[test]
fn order_file() {
    let text = "schema_version = 1\ncolour = 2\n[[format]]\nid = \"c\"\nenabled = false\n\
                [[format]]\nid = \"ghost\"\n[[format]]\nid = \"a\"\nnote = 1\n";
    let (order, warnings) = FormatOrder::parse(text, "order.toml").unwrap();
    let messages: Vec<_> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "order.toml: unknown key `colour`",
            "order.toml: unknown key `format[2].note`"
        ]
    );
    let r = registry(&[("a", "*.a", true), ("b", "*.b", false), ("c", "*.c", true)]);
    let (r, warnings) = r.with_order(&order).unwrap();
    let ids: Vec<_> = r
        .entries()
        .iter()
        .map(|e| (e.id().as_str(), e.enabled))
        .collect();
    assert_eq!(ids, [("c", false), ("a", true), ("b", false)]);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, WarningCode::UnknownConfigKey);
    assert!(warnings[0].message.contains("ghost"));
    let back = FormatOrder::from_registry(&r);
    let (again, w) = FormatOrder::parse(&back.to_toml(), "x").unwrap();
    assert!(w.is_empty());
    assert_eq!(again, back);
    let dup = FormatOrder {
        schema_version: 1,
        formats: vec![
            OrderItem {
                id: id("a"),
                enabled: None,
            },
            OrderItem {
                id: id("a"),
                enabled: None,
            },
        ],
    };
    assert!(matches!(
        r.with_order(&dup),
        Err(FormatError::DuplicateId { .. })
    ));
    let newer = FormatOrder::parse("schema_version = 7\n", "o.toml").unwrap_err();
    assert!(matches!(newer, FormatError::Schema { .. }));
}
