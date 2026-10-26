//! `load_options` (D-25, D-31), `FormatStore::load`/`materialize` (§2.12.4, D-21, D-27) and `write_atomic`
//! (SES-053) — §6.2.8 unit tables.

mod common;

use std::path::Path;

use hikasa_core::WarningCode;
use hikasa_formats::{FormatId, Origin, RegexLimits, ShadowStatus};
use hikasa_session::options::{OPTIONS_SCHEMA_VERSION, parse_options};
use hikasa_session::{
    ConfigLocation, ConfigMode, FormatStore, ProgramOptions, SessionError, load_options,
    write_atomic,
};

fn location(dir: &Path) -> ConfigLocation {
    ConfigLocation {
        dir: dir.to_path_buf(),
        mode: ConfigMode::Explicit,
        cache_dir: dir.join("cache"),
        is_default: false,
    }
}

#[test]
fn missing_options_file_means_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let load = load_options(&location(&tmp.path().join("nothing-here"))).unwrap();
    assert_eq!(load.options, ProgramOptions::default());
    assert_eq!(load.options.context_lines, 3);
    assert_eq!(load.options.binary_buffer_size, 1 << 20);
    assert_eq!(load.options.content_memory_budget_mib, 1024);
    assert!(load.warnings.is_empty() && load.ignored_keys.is_empty());
}

#[test]
fn partial_file_and_known_keys() {
    let tmp = tempfile::tempdir().unwrap();
    common::write(
        tmp.path(),
        "options.toml",
        b"schema_version = 1\n[text_editing]\nnumber_of_context_lines = 7\n\
          [tweaks.file_operations]\ncontent_memory_budget_mib = 64\n",
    );
    let load = load_options(&location(tmp.path())).unwrap();
    assert_eq!(load.options.context_lines, 7);
    assert_eq!(load.options.content_memory_budget_mib, 64);
    assert_eq!(
        load.options.binary_buffer_size,
        1 << 20,
        "missing keys keep defaults"
    );
    assert!(load.warnings.is_empty());
    assert!(load.ignored_keys.is_empty());
    let all = parse_options(
        "[tweaks.file_operations]\nbuffer_size_for_binary_compare = 65536\n",
        "o",
    )
    .unwrap();
    assert_eq!(all.options.binary_buffer_size, 65536);
}

#[test]
fn unknown_keys_inside_known_sections_are_silent_but_listed() {
    let load = parse_options(
        "[text_editing]\nauto_indent = true\nnumber_of_context_lines = 2\n\
         [tweaks]\nbeep = true\n[tweaks.file_operations]\ncomparison_priority = \"idle\"\n\
         [startup]\n[backups]\nmode = 3\n",
        "options.toml",
    )
    .unwrap();
    assert_eq!(load.options.context_lines, 2);
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    assert_eq!(
        load.ignored_keys,
        vec![
            "backups.mode",
            "text_editing.auto_indent",
            "tweaks.beep",
            "tweaks.file_operations.comparison_priority",
        ]
    );
}

#[test]
fn unknown_top_level_section_warns() {
    let load = parse_options("stray = 5\n[colours]\nred = 1\n[tabs]\nx = 1\n", "o.toml").unwrap();
    let codes: Vec<_> = load.warnings.iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        vec![WarningCode::UnknownConfigKey, WarningCode::UnknownConfigKey]
    );
    assert!(load.warnings[0].message.contains("`colours`"));
    assert!(load.warnings[1].message.contains("`stray`"));
    assert_eq!(load.ignored_keys, vec!["colours.red", "stray", "tabs.x"]);
}

#[test]
fn newer_schema_and_bad_values_are_errors() {
    let newer = format!("schema_version = {}\n", OPTIONS_SCHEMA_VERSION + 1);
    let err = parse_options(&newer, "o.toml").unwrap_err();
    assert!(matches!(err, SessionError::Schema { .. }), "{err:?}");
    assert_eq!(
        hikasa_core::Classify::kind(&err),
        hikasa_core::ErrorKind::Unsupported
    );
    assert!(
        parse_options("schema_version = 0\n", "o").is_ok(),
        "older is migrated"
    );
    for bad in [
        "schema_version = \"1\"\n",
        "[text_editing]\nnumber_of_context_lines = -1\n",
        "[text_editing]\nnumber_of_context_lines = \"3\"\n",
        "[tweaks.file_operations]\ncontent_memory_budget_mib = 9999999999\n",
        "not toml at all [",
    ] {
        assert!(
            matches!(parse_options(bad, "o"), Err(SessionError::Config { .. })),
            "{bad}"
        );
    }
}

fn store(dir: &Path) -> FormatStore {
    FormatStore::new(&location(dir))
}

fn id(s: &str) -> FormatId {
    FormatId::new(s).unwrap()
}

#[test]
fn format_store_without_user_files_is_the_builtin_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let (registry, warnings) = store(tmp.path()).load(RegexLimits::default()).unwrap();
    assert!(warnings.is_empty());
    let builtin = hikasa_formats::FormatRegistry::builtin().unwrap();
    let ids = |r: &hikasa_formats::FormatRegistry| {
        r.entries()
            .iter()
            .map(|e| e.id().clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&registry), ids(&builtin));
    assert!(
        !tmp.path().join("formats").exists(),
        "loading never writes (D-21)"
    );
}

#[test]
fn user_formats_override_add_order_and_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let formats = tmp.path().join("formats");
    // Overrides the built-in C format (same id), keeps its position.
    let builtin = hikasa_formats::FormatRegistry::builtin().unwrap();
    let c_position = builtin
        .entries()
        .iter()
        .position(|e| e.id().as_str() == "c-cpp")
        .unwrap();
    let mut c = (*builtin.get(&id("c-cpp")).unwrap().format).clone();
    c.name = "My C".into();
    common::write(
        &formats,
        "c-cpp.toml",
        hikasa_formats::to_toml(&c).as_bytes(),
    );
    // A new format.
    common::write(
        &formats,
        "zzz-notes.toml",
        b"schema_version = 1\nid = \"zzz-notes\"\nname = \"Notes\"\nkind = \"text\"\nmask = \"*.notes\"\nbogus = 1\n",
    );
    // Broken files: TOML syntax, wrong stem, a separator in the mask.
    common::write(&formats, "broken.toml", b"this is [not toml");
    common::write(
        &formats,
        "stem.toml",
        b"schema_version = 1\nid = \"other\"\nname = \"X\"\nkind = \"text\"\n",
    );
    common::write(
        &formats,
        "sep.toml",
        b"schema_version = 1\nid = \"sep\"\nname = \"S\"\nkind = \"text\"\nmask = \"a/*.x\"\n",
    );
    // Not a format file.
    common::write(&formats, "readme.txt", b"ignored");
    let (registry, warnings) = store(tmp.path()).load(RegexLimits::default()).unwrap();
    let entries = registry.entries();
    assert_eq!(entries[c_position].id().as_str(), "c-cpp");
    assert_eq!(entries[c_position].format.name, "My C");
    assert!(
        matches!(&entries[c_position].origin, Origin::User { path } if path.ends_with("c-cpp.toml"))
    );
    let last = entries.last().unwrap();
    assert_eq!(last.id().as_str(), "zzz-notes");
    assert!(last.enabled);
    let skipped: Vec<_> = warnings
        .iter()
        .filter(|w| w.code == WarningCode::UserFormatSkipped)
        .map(|w| w.message.clone())
        .collect();
    assert_eq!(skipped.len(), 3, "{warnings:?}");
    assert!(skipped[0].contains("broken.toml"));
    assert!(skipped[1].contains("sep.toml"));
    assert!(skipped[2].contains("stem.toml"));
    assert!(
        warnings
            .iter()
            .any(|w| w.code == WarningCode::UnknownConfigKey && w.message.contains("bogus")),
        "{warnings:?}"
    );

    // order.toml: listed ids first (an unknown id warns), enabled flags from the list.
    common::write(
        &formats,
        "order.toml",
        b"schema_version = 1\n[[format]]\nid = \"zzz-notes\"\n[[format]]\nid = \"nope\"\n\
          [[format]]\nid = \"c-cpp\"\nenabled = false\n",
    );
    let (registry, warnings) = store(tmp.path()).load(RegexLimits::default()).unwrap();
    let entries = registry.entries();
    assert_eq!(entries[0].id().as_str(), "zzz-notes");
    assert_eq!(entries[1].id().as_str(), "c-cpp");
    assert!(!entries[1].enabled);
    assert!(
        warnings
            .iter()
            .any(|w| w.code == WarningCode::UnknownConfigKey && w.message.contains("nope"))
    );
    let shadow = registry.shadow_report(&Default::default());
    assert_eq!(shadow[1].1, ShadowStatus::Disabled);

    // A duplicate id in order.toml is reported, not resolved.
    common::write(
        &formats,
        "order.toml",
        b"schema_version = 1\n[[format]]\nid = \"c-cpp\"\n[[format]]\nid = \"c-cpp\"\n",
    );
    assert!(matches!(
        store(tmp.path()).load(RegexLimits::default()),
        Err(SessionError::Format(_))
    ));
    // A newer order.toml is an error (D-31).
    common::write(&formats, "order.toml", b"schema_version = 2\n");
    let err = store(tmp.path()).load(RegexLimits::default()).unwrap_err();
    assert_eq!(
        hikasa_core::Classify::kind(&err),
        hikasa_core::ErrorKind::Unsupported
    );
}

#[test]
fn materialize_writes_once_and_never_overwrites() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    store.materialize().unwrap();
    let builtin = hikasa_formats::FormatRegistry::builtin().unwrap();
    for entry in builtin.entries() {
        assert!(
            store.dir().join(format!("{}.toml", entry.id())).is_file(),
            "{}",
            entry.id()
        );
    }
    let order = std::fs::read_to_string(store.dir().join("order.toml")).unwrap();
    assert!(order.contains("schema_version = 1"));
    // Materialised files load as user formats in the built-in order, with the same enabled flags (D-27).
    let (registry, warnings) = store.load(RegexLimits::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let pairs = |r: &hikasa_formats::FormatRegistry| {
        r.entries()
            .iter()
            .map(|e| (e.id().clone(), e.enabled))
            .collect::<Vec<_>>()
    };
    assert_eq!(pairs(&registry), pairs(&builtin));
    assert!(
        registry
            .entries()
            .iter()
            .all(|e| matches!(e.origin, Origin::User { .. }))
    );
    // Never overwrites.
    let c = store.dir().join("c-cpp.toml");
    std::fs::write(&c, "# mine\n").unwrap();
    std::fs::write(store.dir().join("order.toml"), "schema_version = 1\n").unwrap();
    store.materialize().unwrap();
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "# mine\n");
    assert_eq!(
        std::fs::read_to_string(store.dir().join("order.toml")).unwrap(),
        "schema_version = 1\n"
    );
}

#[test]
fn write_atomic_replaces_and_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("a.toml");
    write_atomic(&target, b"one").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"one");
    write_atomic(&target, b"two").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"two");
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("a.toml")]);
    // A missing folder is an error and leaves nothing behind.
    let err = write_atomic(&tmp.path().join("missing/x"), b"z").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    // A folder as the target: the rename fails, the target is untouched, no temporary file remains.
    let dir_target = tmp.path().join("d");
    std::fs::create_dir(&dir_target).unwrap();
    std::fs::write(dir_target.join("keep"), b"k").unwrap();
    assert!(write_atomic(&dir_target, b"x").is_err());
    assert_eq!(std::fs::read(dir_target.join("keep")).unwrap(), b"k");
    let names: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains("hikasa-tmp")), "{names:?}");
    // No file name.
    assert!(write_atomic(Path::new("/"), b"x").is_err());
}
