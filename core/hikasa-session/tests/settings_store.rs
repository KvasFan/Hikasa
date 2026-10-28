//! The settings store (M2 W1, contract §6.1): round trips of every schema (A1.1), preservation of unknown keys
//! (A1.2), the migration chain (A1.3), strict and lenient loading (A1.4), atomic writes and conflicts (A1.5),
//! change detection (A1.6), every key of docs/15 (A1.7) and scheme selection (A1.8).

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime};

use hikasa_core::{Classify, ErrorKind, WarningCode};
use hikasa_session::api::ApiOp;
use hikasa_session::commands::{VIEWS, catalogue, view_sections};
use hikasa_session::settings::{
    ArchiveTypes, ChangeKind, Colour, CommandOverrides, ConfigWatch, FileId, FilterPresets,
    KeySegment, LoadMode, OpenWith, Options, Overwrite, Scheme, SchemeFile, SettingsFile,
    SettingsSchema, SettingsStore, State, ThemeSelection, ValueKind, key_infos, parse_key,
};
use hikasa_session::{ApiError, ConfigLocation, ConfigMode, SessionError};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

fn location(dir: &Path) -> ConfigLocation {
    ConfigLocation {
        dir: dir.to_path_buf(),
        mode: ConfigMode::Explicit,
        cache_dir: dir.join("cache"),
        is_default: false,
    }
}

// ---- A1.1 round trips --------------------------------------------------------------------------------------

/// A small deterministic generator driven by a proptest seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    fn chance(&mut self) -> bool {
        self.below(3) == 0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    fn text(&mut self) -> String {
        const ALPHABET: &[&str] = &[
            "a", "Z", "0", " ", "_", "-", ".", "\"", "\\", "/", "é", "漢", "\t", "'", "#",
        ];
        (0..self.below(8)).map(|_| *self.pick(ALPHABET)).collect()
    }

    fn colour(&mut self) -> String {
        let alpha = if self.chance() {
            format!("{:02x}", self.below(256))
        } else {
            String::new()
        };
        format!("#{:06x}{alpha}", self.below(1 << 24))
    }
}

/// A valid random value of `kind` for the key `key` (default `default`).
fn random_value(rng: &mut Rng, key: &str, kind: ValueKind, default: &Value) -> Value {
    const MASKS: &[&str] = &["", "*.c", "*.a;*.b", "-*.bak;*", "a?c.*"];
    match kind {
        ValueKind::Bool => Value::Boolean(rng.below(2) == 1),
        ValueKind::Int { min, max } => {
            let span = (max as i128 - min as i128 + 1).min(1 << 40) as u64;
            Value::Integer(min + rng.below(span) as i64)
        }
        ValueKind::Float { min, max } => {
            let steps = ((max - min) * 2.0) as u64;
            Value::Float(min + rng.below(steps + 1) as f64 / 2.0)
        }
        ValueKind::Enum(values) => Value::String((*rng.pick(values)).to_owned()),
        ValueKind::Mask => Value::String((*rng.pick(MASKS)).to_owned()),
        ValueKind::Colour => Value::String(rng.colour()),
        ValueKind::String | ValueKind::Path if key.ends_with("_scheme") => {
            Value::String((*rng.pick(&["light", "dark", "mine"])).to_owned())
        }
        ValueKind::String | ValueKind::Path | ValueKind::Shortcut => Value::String(rng.text()),
        ValueKind::List if key.ends_with("fallback") => {
            Value::Array((0..8).map(|_| Value::String(rng.colour())).collect())
        }
        ValueKind::List => match default {
            Value::Array(items) if items.iter().all(Value::is_str) => Value::Array(
                (0..rng.below(4))
                    .map(|_| Value::String(rng.text()))
                    .collect(),
            ),
            other => other.clone(),
        },
        ValueKind::Table => default.clone(),
    }
}

/// Sets `value` at the table path `path` (creating tables).
fn put(root: &mut Table, path: &[KeySegment], value: Value) {
    let (last, parents) = path.split_last().unwrap();
    let mut node = root;
    for segment in parents {
        node = node
            .entry(segment.as_key())
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .unwrap();
    }
    node.insert(last.as_key(), value);
}

/// A random tree of `file`: a third of its keys changed, plus random entries of its maps and arrays of tables.
fn random_tree(file: &FileId, rng: &mut Rng) -> Table {
    let mut root = Table::new();
    for info in key_infos(file) {
        if rng.chance() {
            let key = parse_key(&info.key).unwrap();
            let value = random_value(rng, &info.key, info.kind, &info.default);
            put(&mut root, &key.path, value);
        }
    }
    match file {
        FileId::Commands => {
            for _ in 0..rng.below(3) {
                let view = *rng.pick(&VIEWS);
                let section = catalogue().section(rng.pick(view_sections(view))).unwrap();
                let command = rng.pick(&section.commands);
                let shortcuts: Vec<Value> = (0..rng.below(3))
                    .map(|_| {
                        let any = catalogue()
                            .entries()
                            .map(|(_, c)| c.shortcut.clone())
                            .filter(|s| !s.is_empty())
                            .nth(rng.below(50) as usize)
                            .unwrap();
                        Value::String(any)
                    })
                    .collect();
                let base = vec![
                    KeySegment::Name("view".into()),
                    KeySegment::Name(view.into()),
                ];
                let mut at = base.clone();
                at.extend([
                    KeySegment::Name("command".into()),
                    KeySegment::Name(command.id.clone()),
                ]);
                let mut entry = at.clone();
                entry.push(KeySegment::Name("shortcuts".into()));
                put(&mut root, &entry, Value::Array(shortcuts));
                let mut menu = at.clone();
                menu.push(KeySegment::Name("menu".into()));
                put(&mut root, &menu, Value::Boolean(rng.below(2) == 0));
                let mut large = base;
                large.push(KeySegment::Name("large_buttons".into()));
                put(&mut root, &large, Value::Boolean(true));
            }
        }
        FileId::Filters if rng.below(2) == 0 => {
            let presets: Vec<Value> = (0..rng.below(4))
                .map(|k| {
                    let mut t = Table::new();
                    t.insert("name".into(), Value::String(format!("p{k}{}", rng.text())));
                    t.insert("mask".into(), Value::String("*.x;*.y".into()));
                    if rng.chance() {
                        t.insert("future".into(), Value::Integer(7));
                    }
                    Value::Table(t)
                })
                .collect();
            root.insert("preset".into(), Value::Array(presets));
        }
        FileId::OpenWith => {
            let apps: Vec<Value> = (0..rng.below(3))
                .map(|_| {
                    let mut t = Table::new();
                    t.insert(
                        "description".into(),
                        Value::String(format!("d{}", rng.text())),
                    );
                    t.insert(
                        "command".into(),
                        Value::String("\"my tool\" %f %f2 +%l".into()),
                    );
                    t.insert("shortcut".into(), Value::String("Ctrl+Alt+E".into()));
                    t.insert(
                        "working_folder".into(),
                        Value::String((*rng.pick(&["parent", "base", "/srv/x"])).into()),
                    );
                    t.insert(
                        "path_delimiter".into(),
                        Value::String((*rng.pick(&["", "/", "\\"])).into()),
                    );
                    t.insert("accepts_folders".into(), Value::Boolean(rng.below(2) == 0));
                    if rng.chance() {
                        t.insert("future".into(), Value::String(rng.text()));
                    }
                    Value::Table(t)
                })
                .collect();
            root.insert("app".into(), Value::Array(apps));
        }
        FileId::State => {
            let mut dialog = Table::new();
            dialog.insert("last".into(), Value::String(rng.text()));
            dialog.insert("n".into(), Value::Integer(rng.below(9) as i64));
            let mut dialogs = Table::new();
            dialogs.insert("report".into(), Value::Table(dialog));
            root.insert("dialogs".into(), Value::Table(dialogs));
        }
        FileId::Themes(_) if rng.below(2) == 0 => {
            let mut style = Table::new();
            style.insert("color".into(), Value::String(rng.colour()));
            style.insert("italic".into(), Value::Boolean(true));
            put(
                &mut root,
                &[
                    KeySegment::Name("file".into()),
                    KeySegment::Name("syntax".into()),
                    KeySegment::Name("label".into()),
                ],
                Value::Table(style),
            );
        }
        _ => {}
    }
    root
}

/// `parse(to_toml(v)) == v` for the value of a random tree.
fn round_trip<T: SettingsSchema>(tree: Table) -> Result<(), TestCaseError> {
    round_trip_as::<T>(tree, |a, b| a == b)
}

/// A scheme file is written sparsely against the built-in of its base (D-78): `parse(to_toml(v))` resolves to
/// the same scheme as `v`.
fn round_trip_scheme(tree: Table) -> Result<(), TestCaseError> {
    round_trip_as::<SchemeFile>(tree, |a, b| {
        Scheme::resolve("x", Some(a)) == Scheme::resolve("x", Some(b))
    })
}

/// The round trip with `same` deciding whether the value read back is the value written.
fn round_trip_as<T: SettingsSchema>(
    tree: Table,
    same: impl Fn(&T, &T) -> bool,
) -> Result<(), TestCaseError> {
    let value: T = Value::Table(tree.clone())
        .try_into()
        .map_err(|e| TestCaseError::fail(format!("{e}: {tree:?}")))?;
    prop_assert!(value.validate().is_empty(), "{:?}", value.validate());
    let mut file = SettingsFile::<T>::parse("", "x.toml", LoadMode::Strict).unwrap();
    *file.value_mut() = value.clone();
    let text = file.to_toml();
    prop_assert!(text.starts_with("schema_version = 1\n"), "{text}");
    let back = SettingsFile::<T>::parse(&text, "x.toml", LoadMode::Strict)
        .map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
    prop_assert!(
        same(back.value(), &value),
        "{:?} != {:?}\n{}",
        back.value(),
        value,
        text
    );
    prop_assert!(
        back.loaded().warnings.is_empty(),
        "{:?}",
        back.loaded().warnings
    );
    prop_assert_eq!(back.to_toml(), text);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_schema_round_trips(seed in 1u64..) {
        let mut rng = Rng(seed);
        round_trip::<Options>(random_tree(&FileId::Options, &mut rng))?;
        round_trip::<State>(random_tree(&FileId::State, &mut rng))?;
        round_trip::<CommandOverrides>(random_tree(&FileId::Commands, &mut rng))?;
        round_trip::<ThemeSelection>(random_tree(&FileId::Theme, &mut rng))?;
        round_trip_scheme(random_tree(&FileId::Themes("dark".into()), &mut rng))?;
        round_trip_scheme(random_tree(&FileId::Themes("mine".into()), &mut rng))?;
        round_trip::<FilterPresets>(random_tree(&FileId::Filters, &mut rng))?;
        round_trip::<OpenWith>(random_tree(&FileId::OpenWith, &mut rng))?;
        round_trip::<ArchiveTypes>(random_tree(&FileId::Archives, &mut rng))?;
    }
}

/// Defaults write only the version, and a second save writes nothing.
fn defaults_and_save_once<T: SettingsSchema>(dir: &Path, change: impl FnOnce(&mut T)) {
    let path = dir.join(T::FILE.relative_path().replace("themes/", "themes/s-"));
    let mut file = SettingsFile::<T>::load(&path, LoadMode::Strict).unwrap();
    assert_eq!(file.to_toml(), "schema_version = 1\n", "{path:?}");
    assert!(
        !file.save(Overwrite::IfUnchanged).unwrap(),
        "no file of defaults"
    );
    assert!(!path.exists());
    change(file.value_mut());
    assert!(file.save(Overwrite::IfUnchanged).unwrap(), "{path:?}");
    assert!(
        !file.save(Overwrite::IfUnchanged).unwrap(),
        "{path:?} written twice"
    );
    let again = SettingsFile::<T>::load(&path, LoadMode::Strict).unwrap();
    assert_eq!(again.value(), file.value());
}

#[test]
fn defaults_write_nothing_and_saves_write_once() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    defaults_and_save_once::<Options>(d, |o| o.tabs.hide_tab_bar_if_single_tab = true);
    defaults_and_save_once::<State>(d, |s| s.last_session = "s".into());
    defaults_and_save_once::<CommandOverrides>(d, |c| {
        c.view.entry("home".into()).or_default().large_buttons = true;
    });
    defaults_and_save_once::<ThemeSelection>(d, |t| t.fonts.hex.size = 9.5);
    defaults_and_save_once::<SchemeFile>(d, |s| {
        s.picture.same = Some(hikasa_session::settings::Colour::hex(1))
    });
    defaults_and_save_once::<FilterPresets>(d, |f| f.preset.truncate(1));
    defaults_and_save_once::<OpenWith>(d, |o| {
        o.app.push(hikasa_session::settings::OpenWithApp {
            description: "x".into(),
            command: "x %f".into(),
            ..Default::default()
        });
    });
    defaults_and_save_once::<ArchiveTypes>(d, |a| a.types.zst = String::new());
}

// ---- A1.2 preservation -------------------------------------------------------------------------------------

#[test]
fn unknown_keys_survive_a_set() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let options = "schema_version = 1\nfuture_top = 3\n\n[text_editing]\nauto_indent = false\nwhat = \"kept\"\n\n\
                   [tweaks.metrics]\nmax_mru_items = 7\nzzz = [1, 2]\n\n[archive_types]\nlegacy = true\n\n\
                   [colours.deep]\nred = \"#ff0000\"\n";
    let filters = "schema_version = 1\n\n[[preset]]\nname = \"A\"\nmask = \"*.a\"\nicon = \"star\"\n\n\
                   [[preset]]\nname = \"B\"\nmask = \"*.b\"\n\n[extra]\nx = 1\n";
    let openwith = "schema_version = 1\n\n[[app]]\ndescription = \"Ed\"\ncommand = \"ed %f\"\nnew_flag = true\n";
    common::write(&config, "options.toml", options.as_bytes());
    common::write(&config, "filters.toml", filters.as_bytes());
    common::write(&config, "openwith.toml", openwith.as_bytes());
    let service = common::service(tmp.path());
    let table = |name: &str| -> Table {
        toml::from_str(&std::fs::read_to_string(config.join(name)).unwrap()).unwrap()
    };
    let before = table("options.toml");
    let dto = service
        .settings_set(
            &parse_key("options.tabs.open_sessions_in").unwrap(),
            "window",
            false,
        )
        .unwrap();
    assert!(dto.written);
    let mut after = table("options.toml");
    let tabs = after.remove("tabs").unwrap();
    assert_eq!(
        tabs.get("open_sessions_in"),
        Some(&Value::String("window".into()))
    );
    assert_eq!(after, before, "only the set key changed");

    for (name, key, value) in [
        ("filters.toml", "filters.preset.1.mask", "*.bb"),
        ("openwith.toml", "openwith.app.0.description", "Editor"),
    ] {
        let before = table(name);
        service
            .settings_set(&parse_key(key).unwrap(), value, false)
            .unwrap();
        let after = table(name);
        let path = parse_key(key).unwrap().path;
        let mut expected = Value::Table(before);
        let mut node = &mut expected;
        for segment in &path[..path.len() - 1] {
            node = match segment {
                KeySegment::Name(n) => node.get_mut(n).unwrap(),
                KeySegment::Index(i) => node.get_mut(*i).unwrap(),
            };
        }
        node.as_table_mut()
            .unwrap()
            .insert(path.last().unwrap().as_key(), Value::String(value.into()));
        assert_eq!(Value::Table(after), expected, "{name}");
    }
}

// ---- A1.3 migration ----------------------------------------------------------------------------------------

/// A test schema at version 3: v1 had `title` and `width`; v2 renamed `title` to `name`; v3 replaced `width` by
/// `size = width * 10`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct V3 {
    name: String,
    size: i64,
}

impl SettingsSchema for V3 {
    const FILE: FileId = FileId::State;
    const VERSION: u32 = 3;

    fn migrate(from: u32, table: &mut Table) -> Result<(), String> {
        match from {
            1 => {
                let title = table.remove("title").ok_or("no title")?;
                table.insert("name".into(), title);
                Ok(())
            }
            2 => {
                let width = table
                    .remove("width")
                    .and_then(|w| w.as_integer())
                    .ok_or("no width")?;
                table.insert("size".into(), Value::Integer(width * 10));
                Ok(())
            }
            other => Err(format!("no migration from {other}")),
        }
    }

    fn validate(&self) -> Vec<hikasa_session::settings::Problem> {
        Vec::new()
    }
}

#[test]
fn migration_chain_reads_old_files_and_writes_back_on_save() {
    let tmp = tempfile::tempdir().unwrap();
    let v1 = common::write(
        tmp.path(),
        "v1.toml",
        b"schema_version = 1\ntitle = \"x\"\nwidth = 2\n",
    );
    let v2 = common::write(
        tmp.path(),
        "v2.toml",
        b"schema_version = 2\nname = \"x\"\nwidth = 2\n",
    );
    let expected = V3 {
        name: "x".into(),
        size: 20,
    };
    for (path, from) in [(&v1, 1), (&v2, 2)] {
        let original = std::fs::read(path).unwrap();
        for mode in [LoadMode::Strict, LoadMode::Lenient] {
            let file = SettingsFile::<V3>::load(path, mode).unwrap();
            assert_eq!(file.value(), &expected);
            assert_eq!(file.loaded().migrated_from, Some(from));
        }
        assert_eq!(
            std::fs::read(path).unwrap(),
            original,
            "not written at load"
        );
        let mut file = SettingsFile::<V3>::load(path, LoadMode::Strict).unwrap();
        assert!(file.save(Overwrite::IfUnchanged).unwrap());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "schema_version = 3\nname = \"x\"\nsize = 20\n"
        );
        assert_eq!(file.loaded().migrated_from, None);
    }
    let broken = common::write(tmp.path(), "b.toml", b"schema_version = 1\nname = \"x\"\n");
    let err = SettingsFile::<V3>::load(&broken, LoadMode::Lenient).unwrap_err();
    assert!(matches!(err, SessionError::Config { .. }), "{err}");
    for mode in [LoadMode::Strict, LoadMode::Lenient] {
        let err = SettingsFile::<V3>::parse("schema_version = 4\n", "n", mode).unwrap_err();
        assert!(matches!(err, SessionError::Schema { .. }), "{err}");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        for bad in [
            "schema_version = -1\n",
            "schema_version = \"3\"\n",
            "schema_version = 2.5\n",
        ] {
            let err = SettingsFile::<V3>::parse(bad, "n", mode).unwrap_err();
            assert!(matches!(err, SessionError::Config { .. }), "{bad}: {err}");
        }
    }
    // A file without a version is current.
    let file = SettingsFile::<V3>::parse("name = \"y\"\n", "n", LoadMode::Strict).unwrap();
    assert_eq!(file.loaded().migrated_from, None);
}

// ---- A1.4 strict and lenient -------------------------------------------------------------------------------

#[test]
fn bad_values_fail_strictly_and_are_repaired_leniently() {
    let text = "[text_editing]\nnumber_of_context_lines = \"x\"\nauto_indent = 3\n\n[tweaks.metrics]\nmax_mru_items = 0\n\
                hint_show_ms = 99999\n[appearance]\ntheme_mode = \"pink\"\n";
    let err = SettingsFile::<Options>::parse(text, "o.toml", LoadMode::Strict).unwrap_err();
    let message = err.to_string();
    assert!(
        message.starts_with("o.toml: `appearance.theme_mode`: unknown variant `pink`"),
        "{message}"
    );
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    let file = SettingsFile::<Options>::parse(text, "o.toml", LoadMode::Lenient).unwrap();
    assert_eq!(file.value(), &Options::default());
    let warnings: Vec<_> = file
        .loaded()
        .warnings
        .iter()
        .map(|w| (w.code, w.message.clone()))
        .collect();
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|(c, _)| *c == WarningCode::InvalidConfigValue)
    );
    for key in [
        "appearance.theme_mode",
        "text_editing.auto_indent",
        "text_editing.number_of_context_lines",
        "tweaks.metrics.hint_show_ms",
        "tweaks.metrics.max_mru_items",
    ] {
        assert!(
            warnings.iter().any(|(_, m)| m.contains(key)),
            "{key}: {warnings:?}"
        );
    }
    // A strict range error names the key too.
    let err = SettingsFile::<Options>::parse(
        "[tweaks.metrics]\nmax_mru_items = 0\n",
        "o",
        LoadMode::Strict,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("`tweaks.metrics.max_mru_items`: must be from 1 to 100"),
        "{err}"
    );
    // The repaired file writes the defaults (the bad keys are gone).
    assert_eq!(file.to_toml(), "schema_version = 1\n");
}

// ---- A1.5 atomic writes and conflicts ----------------------------------------------------------------------

#[test]
fn failed_writes_leave_the_target_and_no_temporary_file() {
    let tmp = tempfile::tempdir().unwrap();
    // The target is a folder.
    let target = tmp.path().join("state.toml");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep"), "k").unwrap();
    let mut file = SettingsFile::<State>::new_default(&target);
    file.value_mut().last_session = "x".into();
    assert!(file.save(Overwrite::Force).is_err());
    assert_eq!(std::fs::read_to_string(target.join("keep")).unwrap(), "k");
    let names = |dir: &Path| -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    };
    assert!(names(tmp.path()).iter().all(|n| !n.contains("hikasa-tmp")));
    // A read-only folder (skipped where the permission is not enforced, e.g. as root).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ro = tmp.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        let path = ro.join("options.toml");
        std::fs::write(&path, "schema_version = 1\n").unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(ro.join("probe"), "p").is_err() {
            let mut file = SettingsFile::<Options>::load(&path, LoadMode::Strict).unwrap();
            file.value_mut().tabs.hide_tab_bar_if_single_tab = true;
            let err = file.save(Overwrite::IfUnchanged).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "schema_version = 1\n"
            );
            assert_eq!(names(&ro), vec!["options.toml".to_owned()]);
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn an_external_edit_is_a_conflict_unless_forced() {
    let tmp = tempfile::tempdir().unwrap();
    let path = common::write(tmp.path(), "theme.toml", b"light_scheme = \"light\"\n");
    let mut file = SettingsFile::<ThemeSelection>::load(&path, LoadMode::Strict).unwrap();
    assert!(!file.changed_on_disk().unwrap());
    std::fs::write(&path, "dark_scheme = \"dark\"\n").unwrap();
    assert!(file.changed_on_disk().unwrap());
    file.value_mut().fonts.editor.size = 20.0;
    let err = file.save(Overwrite::IfUnchanged).unwrap_err();
    assert!(matches!(err, SessionError::Conflict { .. }), "{err}");
    assert!(
        err.to_string()
            .ends_with("changed on disk since it was read")
    );
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(ApiError::from(err).exit_code(ApiOp::Other).code(), 100);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "dark_scheme = \"dark\"\n"
    );
    assert!(file.save(Overwrite::Force).unwrap());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("size = 20.0")
    );
    // A file removed since the load is a conflict as well.
    std::fs::remove_file(&path).unwrap();
    file.value_mut().fonts.editor.size = 21.0;
    assert!(matches!(
        file.save(Overwrite::IfUnchanged),
        Err(SessionError::Conflict { .. })
    ));
}

// ---- A1.6 change detection ---------------------------------------------------------------------------------

#[test]
fn the_watch_reports_created_changed_removed_and_format_files() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.toml");
    let b = common::write(tmp.path(), "b.toml", b"one");
    let formats = tmp.path().join("formats");
    std::fs::create_dir(&formats).unwrap();
    let mut watch = ConfigWatch::new([a.clone(), b.clone()]);
    watch.track_folder(formats.clone());
    let kinds = |watch: &mut ConfigWatch| -> Vec<(String, ChangeKind)> {
        watch
            .poll()
            .into_iter()
            .map(|c| {
                (
                    c.path.file_name().unwrap().to_string_lossy().into_owned(),
                    c.kind,
                )
            })
            .collect()
    };
    assert!(kinds(&mut watch).is_empty(), "nothing on an unchanged poll");
    std::fs::write(&a, "x").unwrap();
    assert_eq!(
        kinds(&mut watch),
        vec![("a.toml".into(), ChangeKind::Created)]
    );
    std::fs::write(&b, "three").unwrap();
    assert_eq!(
        kinds(&mut watch),
        vec![("b.toml".into(), ChangeKind::Changed)]
    );
    // Same length and the same modification time, other bytes: the CRC finds it.
    let mtime = std::fs::metadata(&b).unwrap().modified().unwrap();
    std::fs::write(&b, "THREE").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&b)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(std::fs::metadata(&b).unwrap().modified().unwrap(), mtime);
    assert_eq!(
        kinds(&mut watch),
        vec![("b.toml".into(), ChangeKind::Changed)]
    );
    std::fs::remove_file(&a).unwrap();
    assert_eq!(
        kinds(&mut watch),
        vec![("a.toml".into(), ChangeKind::Removed)]
    );
    let x = common::write(&formats, "x.toml", b"schema_version = 1\n");
    common::write(&formats, "note.txt", b"not a format");
    assert_eq!(
        kinds(&mut watch),
        vec![("x.toml".into(), ChangeKind::Created)]
    );
    std::fs::remove_file(&x).unwrap();
    assert_eq!(
        kinds(&mut watch),
        vec![("x.toml".into(), ChangeKind::Removed)]
    );
    // Our own save is acknowledged.
    std::fs::write(&b, "ours").unwrap();
    watch.acknowledge(&b);
    assert!(kinds(&mut watch).is_empty());
    // An older modification time is a change too.
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&b)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(
        kinds(&mut watch),
        vec![("b.toml".into(), ChangeKind::Changed)]
    );
    // The store's watch covers every settings file, the schemes and the formats.
    let store = SettingsStore::new(&location(tmp.path()));
    let mut watch = store.watch();
    assert!(watch.paths().any(|p| p.ends_with("options.toml")));
    common::write(tmp.path(), "themes/mine.toml", b"base = \"dark\"\n");
    common::write(tmp.path(), "openwith.toml", b"");
    let changed: Vec<_> = watch.poll().into_iter().map(|c| c.path).collect();
    assert_eq!(
        changed,
        vec![
            tmp.path().join("openwith.toml"),
            tmp.path().join("themes/mine.toml")
        ]
    );
}

// ---- A1.7 every key of docs/15 -----------------------------------------------------------------------------

/// The backtick words of a cell outside parentheses.
fn ticked(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '`' => {
                let word: String = chars.by_ref().take_while(|c| *c != '`').collect();
                if depth == 0 {
                    out.push(word);
                }
            }
            _ => {}
        }
    }
    out
}

/// One documented key: (dotted key, kind, default).
fn documented_keys() -> Vec<(String, ValueKind, Value)> {
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/15-options-reference.md"
    ))
    .unwrap();
    let section = doc.split("## Keys (M2").nth(1).unwrap();
    let mut keys = Vec::new();
    for line in section.lines().filter(|l| l.starts_with("| OPT-")) {
        let cells: Vec<&str> = line.split(" | ").collect();
        let (key_cell, type_cell, default_cell) =
            (cells[2], cells[3], cells[4].trim_end_matches(" |"));
        let names = ticked(key_cell);
        let Some(first) = names.first().filter(|n| n.starts_with('[')) else {
            continue;
        };
        let (table, first_key) = first[1..].split_once("] ").unwrap();
        let mut leaf_names = vec![first_key.to_owned()];
        leaf_names.extend(names[1..].iter().cloned());
        let kind = if type_cell.starts_with("bool") {
            ValueKind::Bool
        } else if let Some(range) = type_cell.strip_prefix("int ") {
            let range = range.split_whitespace().next().unwrap();
            let (min, max) = range.split_once("..").unwrap();
            ValueKind::Int {
                min: min.parse().unwrap(),
                max: max.parse().unwrap(),
            }
        } else if type_cell.starts_with("enum") {
            let values: Vec<&'static str> = ticked(type_cell)
                .into_iter()
                .map(|v| &*Box::leak(v.into_boxed_str()))
                .collect();
            ValueKind::Enum(Box::leak(values.into_boxed_slice()))
        } else if type_cell.starts_with("string") {
            ValueKind::String
        } else if type_cell.starts_with("path") {
            ValueKind::Path
        } else {
            panic!("type {type_cell:?}");
        };
        for name in leaf_names {
            let default = if default_cell.contains(" = ") {
                // `compare` = `main_menu`, `compare_to_registry` = `none`, others `submenu`
                let words = ticked(default_cell);
                let mut value = words.last().unwrap().clone();
                for pair in words.chunks(2) {
                    if pair.len() == 2 && pair[0] == name {
                        value.clone_from(&pair[1]);
                    }
                }
                Value::String(value)
            } else {
                let first = default_cell.split_whitespace().next().unwrap();
                match kind {
                    ValueKind::Bool => Value::Boolean(first == "true"),
                    ValueKind::Int { .. } => Value::Integer(first.parse().unwrap()),
                    _ => {
                        let word = ticked(default_cell).into_iter().next().unwrap();
                        Value::String(if word == "\"\"" { String::new() } else { word })
                    }
                }
            };
            keys.push((format!("{table}.{name}"), kind, default));
        }
    }
    keys
}

#[test]
fn every_documented_key_exists_with_its_type_range_and_default() {
    let documented = documented_keys();
    assert!(documented.len() > 90, "{}", documented.len());
    let infos = key_infos(&FileId::Options);
    let sorted = |kind: ValueKind| match kind {
        ValueKind::Enum(values) => {
            let mut v = values.to_vec();
            v.sort_unstable();
            format!("enum {}", v.join("|"))
        }
        other => other.describe(),
    };
    for (key, kind, default) in &documented {
        let full = format!("options.{key}");
        let info = infos
            .iter()
            .find(|i| i.key == full)
            .unwrap_or_else(|| panic!("{full} is documented but not a key"));
        assert_eq!(sorted(info.kind), sorted(*kind), "{full}");
        assert_eq!(&info.default, default, "{full}");
        assert!(info.opt.is_some() && info.priority.is_some(), "{full}");
    }
    for info in &infos {
        assert!(
            documented
                .iter()
                .any(|(k, _, _)| format!("options.{k}") == info.key),
            "{} is a key but not documented",
            info.key
        );
    }
    // Every range at both bounds and one step outside.
    for (key, kind, _) in &documented {
        let ValueKind::Int { min, max } = kind else {
            continue;
        };
        let (table, leaf) = key.rsplit_once('.').unwrap();
        let text = |n: i64| format!("[{table}]\n{leaf} = {n}\n");
        for ok in [*min, *max] {
            let file = SettingsFile::<Options>::parse(&text(ok), "o", LoadMode::Strict)
                .unwrap_or_else(|e| panic!("{key} = {ok}: {e}"));
            assert!(file.to_toml().contains(&format!("{leaf} = {ok}")) || ok == info_default(key));
        }
        for bad in [min - 1, max + 1] {
            let err =
                SettingsFile::<Options>::parse(&text(bad), "o", LoadMode::Strict).unwrap_err();
            assert!(err.to_string().contains(leaf), "{key} = {bad}: {err}");
        }
    }
}

/// The default of an integer key.
fn info_default(key: &str) -> i64 {
    key_infos(&FileId::Options)
        .into_iter()
        .find(|i| i.key == format!("options.{key}"))
        .and_then(|i| i.default.as_integer())
        .unwrap()
}

// ---- A1.8 scheme selection -----------------------------------------------------------------------------------

#[test]
fn a_missing_scheme_is_an_error_or_falls_back() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(&location(tmp.path()));
    common::write(
        tmp.path(),
        "theme.toml",
        b"light_scheme = \"mine\"\ndark_scheme = \"gone\"\n",
    );
    common::write(
        tmp.path(),
        "themes/mine.toml",
        b"base = \"dark\"\n[folder]\nbackground = \"#000001\"\n",
    );
    let err = store.theme(LoadMode::Strict).unwrap_err();
    assert!(err.to_string().contains("dark_scheme"), "{err}");
    let loaded = store.theme(LoadMode::Lenient).unwrap();
    assert_eq!(loaded.value.light_scheme, "mine");
    assert_eq!(loaded.value.dark_scheme, "dark");
    assert_eq!(loaded.warnings[0].code, WarningCode::InvalidConfigValue);
    assert_eq!(store.scheme_names(), vec!["dark", "light", "mine"]);
    let (scheme, _) = store.resolved_scheme("mine", LoadMode::Strict).unwrap();
    assert_eq!(scheme.folder.background.to_string(), "#000001");
    let dark = hikasa_session::settings::Scheme::builtin(
        hikasa_session::settings::theme::SchemeBase::Dark,
    );
    assert_eq!(scheme.file, dark.file, "the base fills the rest");
}

// ---- W1 review regressions -----------------------------------------------------------------------------------

/// Three Open With entries; the second has `extra` (one more line).
fn three_apps(extra: &str) -> String {
    format!(
        "schema_version = 1\n\n[[app]]\ndescription = \"A\"\ncommand = \"a %f\"\n\n\
         [[app]]\ndescription = \"B\"\ncommand = \"b %f\"\n{extra}\n\n\
         [[app]]\ndescription = \"C\"\ncommand = \"c %f\"\n"
    )
}

#[test]
fn a_type_error_in_an_entry_names_its_key_and_keeps_every_entry() {
    let text = three_apps("wait_for_previous = \"yes\"");
    let err = SettingsFile::<OpenWith>::parse(&text, "ow", LoadMode::Strict).unwrap_err();
    assert!(
        err.to_string().contains("`app.1.wait_for_previous`"),
        "{err}"
    );
    let file = SettingsFile::<OpenWith>::parse(&text, "ow", LoadMode::Lenient).unwrap();
    let names: Vec<&str> = file
        .value()
        .app
        .iter()
        .map(|a| a.description.as_str())
        .collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert!(!file.value().app[1].wait_for_previous, "the leaf's default");
    let warnings = &file.loaded().warnings;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("app.1.wait_for_previous"));
    // An entry without keys before the bad one keeps the positions.
    let text = "[[app]]\n\n[[app]]\ndescription = \"B\"\ncommand = \"b %f\"\naccepts_files = 3\n";
    let err = SettingsFile::<OpenWith>::parse(text, "ow", LoadMode::Strict).unwrap_err();
    assert!(err.to_string().contains("`app.1.accepts_files`"), "{err}");
    // The same for the presets of filters.toml.
    let text = "[[preset]]\nname = \"A\"\nmask = \"*.a\"\n\n[[preset]]\nname = 7\nmask = \"*.b\"\n\n\
                [[preset]]\nname = \"C\"\nmask = \"*.c\"\n";
    let err = SettingsFile::<FilterPresets>::parse(text, "f", LoadMode::Strict).unwrap_err();
    assert!(err.to_string().contains("`preset.1.name`"), "{err}");
    let file = SettingsFile::<FilterPresets>::parse(text, "f", LoadMode::Lenient).unwrap();
    let masks: Vec<&str> = file
        .value()
        .preset
        .iter()
        .map(|p| p.mask.as_str())
        .collect();
    assert_eq!(masks, ["*.a", "*.b", "*.c"]);
}

#[test]
fn lenient_loading_keeps_an_invalid_entry_at_its_position() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let path = common::write(
        &config,
        "openwith.toml",
        three_apps("command = \"\"")
            .replace("command = \"b %f\"\n", "")
            .as_bytes(),
    );
    let file = SettingsFile::<OpenWith>::load(&path, LoadMode::Lenient).unwrap();
    assert_eq!(file.value().app.len(), 3);
    assert_eq!(file.value().app[1].command, "");
    assert_eq!(
        file.loaded().warnings[0].code,
        WarningCode::InvalidConfigValue
    );
    let message = &file.loaded().warnings[0].message;
    assert!(
        message.contains("`app.1`") && message.ends_with("(the entry is kept as it is)"),
        "{message}"
    );
    let service = common::service(tmp.path());
    let dto = service
        .settings_set(
            &parse_key("openwith.app.1.description").unwrap(),
            "B2",
            false,
        )
        .unwrap();
    assert!(dto.written);
    let back = SettingsFile::<OpenWith>::load(&path, LoadMode::Lenient).unwrap();
    let apps: Vec<(&str, &str)> = back
        .value()
        .app
        .iter()
        .map(|a| (a.description.as_str(), a.command.as_str()))
        .collect();
    assert_eq!(apps, [("A", "a %f"), ("B2", ""), ("C", "c %f")]);
    // Another entry can still be changed; a change that adds a problem cannot.
    service
        .settings_set(
            &parse_key("openwith.app.2.command").unwrap(),
            "cc %f",
            false,
        )
        .unwrap();
    let err = service
        .settings_set(&parse_key("openwith.app.0.command").unwrap(), "", false)
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err}");
    let get = service
        .settings_get(&parse_key("openwith.app.2.command").unwrap())
        .unwrap();
    assert_eq!(get.value, serde_json::json!("cc %f"));
}

#[test]
fn a_scheme_file_is_sparse_against_the_builtin_of_its_new_base() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    let path = common::write(
        &config,
        "themes/mine.toml",
        b"schema_version = 1\n\n[folder]\nbackground = \"#123456\"\n",
    );
    let service = common::service(tmp.path());
    let store = SettingsStore::new(&location(&config));
    service
        .settings_set(&parse_key("themes.mine.base").unwrap(), "dark", false)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "schema_version = 1\nbase = \"dark\"\n\n[folder]\nbackground = \"#123456\"\n"
    );
    let dark = Scheme::resolve("dark", None);
    let (mine, _) = store.resolved_scheme("mine", LoadMode::Strict).unwrap();
    assert_eq!(mine.file, dark.file, "the scheme now resolves to dark");
    // A colour of a dark-based scheme keeps `base`; one equal to the built-in in another case is not written.
    service
        .settings_set(
            &parse_key("themes.mine.folder.stripes").unwrap(),
            &dark.folder.stripes.to_string().to_uppercase(),
            false,
        )
        .unwrap();
    service
        .settings_set(
            &parse_key("themes.mine.folder.selection").unwrap(),
            "#ABCDEF",
            false,
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "schema_version = 1\nbase = \"dark\"\n\n[folder]\nbackground = \"#123456\"\nselection = \"#abcdef\"\n"
    );
    // Back to light: only what the file sets stays.
    service
        .settings_set(&parse_key("themes.mine.base").unwrap(), "light", false)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "schema_version = 1\nbase = \"light\"\n\n[folder]\nbackground = \"#123456\"\nselection = \"#abcdef\"\n"
    );
    // Any save path prunes against the built-in: a file that sets every element of `dark` writes nothing.
    let mut file = SettingsFile::<SchemeFile>::new_default(&config.join("themes/dark.toml"));
    *file.value_mut() = dark.to_file(None);
    assert_eq!(file.to_toml(), "schema_version = 1\n");
    let mut upper = dark.to_file(None);
    upper.folder.background = Some(Colour::parse("#FFFFFF").unwrap());
    *file.value_mut() = upper;
    assert_eq!(
        file.to_toml(),
        "schema_version = 1\n\n[folder]\nbackground = \"#ffffff\"\n"
    );
}

#[test]
fn a_file_modified_before_1970_saves_without_a_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let path = common::write(tmp.path(), "state.toml", b"last_session = \"a\"\n");
    let old = SystemTime::UNIX_EPOCH - Duration::from_secs(86_400 * 365);
    let set = std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|f| f.set_modified(old));
    if set.is_err() || std::fs::metadata(&path).unwrap().modified().unwrap() != old {
        return; // The file system keeps no time before 1970.
    }
    let mut file = SettingsFile::<State>::load(&path, LoadMode::Strict).unwrap();
    assert!(!file.changed_on_disk().unwrap());
    file.value_mut().last_session = "b".into();
    assert!(file.save(Overwrite::IfUnchanged).unwrap());
}

#[test]
fn unknown_keys_under_rule_tables_are_shown_as_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    common::write(
        &config,
        "options.toml",
        b"schema_version = 1\n\n[tabs]\nfuture_flag = true\n",
    );
    let service = common::service(tmp.path());
    let key = parse_key("options.tabs.future_flag").unwrap();
    let get = service.settings_get(&key).unwrap();
    assert_eq!(get.value, serde_json::json!(true));
    assert_eq!(get.default, serde_json::Value::Null);
    let list = service.settings_list(Some(&key), false).unwrap();
    assert_eq!(list.entries.len(), 1);
    assert!(list.entries[0].unknown);
    let err = service.settings_set(&key, "false", false).unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "{err}");
    let err = service
        .settings_get(&parse_key("options.tabs.never_was").unwrap())
        .unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "{err}");
}
