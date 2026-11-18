//! D-317 (M2 W11c; contract §1.11.25, §6.11 A11.23 core): an `importance.elements.<name> = "format"` entry resets an
//! element to the format's own default. The layers merge as before, then such entries are removed before the rules
//! are read: in the defaults, a parent's type and pair overrides and a child session, in every order; in a
//! `--rules-file` too; `session edit --set rules.importance.elements.<name>=format` writes it where an inherited layer
//! holds the element and removes the session's own entry otherwise; any other string is `invalid_input`; every
//! existing session golden reads and writes as before.

mod common;

use std::path::Path;

use common::service;
use hikasa_session::Service;
use hikasa_session::api::{ApiError, SessionEdit, SessionTarget};
use hikasa_session::sessions::{Scope, SessionRef, SessionRules, SessionType, apply_scope};
use hikasa_text::TextRules;

fn r(text: &str) -> SessionRef {
    SessionRef::parse(text).unwrap()
}

/// The state of one layer for element `Comment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Absent,
    Bool(bool),
    Format,
}

const STATES: [Layer; 4] = [
    Layer::Absent,
    Layer::Bool(true),
    Layer::Bool(false),
    Layer::Format,
];

/// The TOML of a layer's `importance.elements` table under `prefix` (`rules`, `override.rules`).
fn elements(prefix: &str, layer: Layer) -> String {
    match layer {
        Layer::Absent => String::new(),
        Layer::Bool(b) => format!("\n[{prefix}.importance.elements]\nComment = {b}\n"),
        Layer::Format => format!("\n[{prefix}.importance.elements]\nComment = \"format\"\n"),
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The importance of `Comment` the effective rules of `child` hold (`None`: no entry, the format's default).
fn effective(service: &Service) -> Option<bool> {
    let session = service.session_effective(&r("child")).expect("effective");
    let SessionRules::Text(rules) = session.rules else {
        panic!("text rules")
    };
    rules.importance.elements.get("Comment").copied()
}

/// Every order of the four layers (defaults → type override → pair override → child): `"format"` drops the entry at
/// its layer, a layer above may set it again. The child's own entry counts where it differs from the defaults (D-82:
/// the child's own changes are what its rules change against them); its `"format"` wins over the overrides wherever
/// the defaults hold the element (the only place a save writes one, §1.11.35 W11c note 2).
#[test]
fn format_drops_the_entry_in_every_layer_order() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("config/sessions");
    let service = service(dir.path());
    let mut checked = 0;
    for defaults in STATES {
        for type_o in STATES {
            for pair_o in STATES {
                for child in STATES {
                    if child == Layer::Format && !matches!(defaults, Layer::Bool(_)) {
                        continue;
                    }
                    write(
                        &sessions.join("_defaults/text-compare.toml"),
                        &format!(
                            "schema_version = 1\ntype = \"text-compare\"\n{}",
                            elements("rules", defaults)
                        ),
                    );
                    let mut parent = String::from(
                        "schema_version = 1\ntype = \"folder-compare\"\n\n[specs]\nleft = \"/l\"\nright = \"/r\"\n",
                    );
                    for (scope, layer, paths) in [
                        ("type", type_o, ""),
                        ("pair", pair_o, "left = \"a.c\"\nright = \"a.c\"\n"),
                    ] {
                        if layer != Layer::Absent {
                            parent.push_str(&format!(
                                "\n[[override]]\nscope = \"{scope}\"\n{paths}type = \"text-compare\"\n{}",
                                elements("override.rules", layer)
                            ));
                        }
                    }
                    write(&sessions.join("parent.toml"), &parent);
                    write(
                        &sessions.join("child.toml"),
                        &format!(
                            "schema_version = 1\ntype = \"text-compare\"\n\n[parent]\nsession = \"parent\"\nleft = \
                             \"a.c\"\nright = \"a.c\"\n{}",
                            elements("rules", child)
                        ),
                    );
                    let mut want = None;
                    for (k, layer) in [defaults, type_o, pair_o, child].into_iter().enumerate() {
                        match layer {
                            Layer::Absent => {}
                            // The child's value equal to the defaults' is no change of its own.
                            Layer::Bool(b) if k == 3 && defaults == Layer::Bool(b) => {}
                            Layer::Bool(b) => want = Some(b),
                            Layer::Format => want = None,
                        }
                    }
                    assert_eq!(
                        effective(&service),
                        want,
                        "defaults {defaults:?}, type {type_o:?}, pair {pair_o:?}, child {child:?}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 4 * 4 * 4 * 4 - 4 * 4 * 2);
}

/// A `--rules-file` reads `"format"` as no entry; another string is refused.
#[test]
fn a_rules_file_reads_format_as_no_entry() {
    let (rules, warnings) =
        TextRules::from_toml("[importance.elements]\nComment = \"format\"\nString = false\n")
            .expect("reads");
    assert!(warnings.is_empty());
    assert_eq!(
        rules.importance.elements.into_iter().collect::<Vec<_>>(),
        [("String".to_owned(), false)]
    );
    let e = TextRules::from_toml("[importance.elements]\nComment = \"never\"\n").unwrap_err();
    assert!(e.to_string().contains("TOML"), "{e}");
}

fn edit(service: &Service, target: &SessionTarget, key: &str, value: &str) -> Result<(), ApiError> {
    service
        .session_edit(
            target,
            &SessionEdit {
                set: vec![(key.to_owned(), value.to_owned())],
                ..SessionEdit::default()
            },
            false,
        )
        .map(|_| ())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// `session edit --set rules.importance.elements.<name>=…`: a name the session does not hold yet is a key; `format`
/// is written over an entry the defaults hold and removes the session's own entry otherwise; another string is
/// `invalid_input`; *Everything Else* is refused.
#[test]
fn session_edit_sets_and_resets_elements() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("config/sessions");
    let service = service(dir.path());
    let named = SessionTarget::Named(r("s"));
    let defaults = SessionTarget::Defaults(SessionType::TextCompare);
    write(
        &sessions.join("s.toml"),
        "schema_version = 1\ntype = \"text-compare\"\n",
    );
    edit(
        &service,
        &named,
        "rules.importance.elements.Comment",
        "false",
    )
    .expect("a new name");
    assert!(read(&sessions.join("s.toml")).contains("Comment = false"));
    // Nothing inherited: the reset removes the session's own entry.
    edit(
        &service,
        &named,
        "rules.importance.elements.Comment",
        "format",
    )
    .expect("reset");
    let text = read(&sessions.join("s.toml"));
    assert!(!text.contains("Comment"), "{text}");
    // The defaults hold it: the reset is written.
    edit(
        &service,
        &defaults,
        "rules.importance.elements.String",
        "false",
    )
    .expect("defaults");
    assert!(read(&sessions.join("_defaults/text-compare.toml")).contains("String = false"));
    edit(
        &service,
        &named,
        "rules.importance.elements.String",
        "format",
    )
    .expect("reset");
    let text = read(&sessions.join("s.toml"));
    assert!(text.contains("String = \"format\""), "{text}");
    let loaded = service.session_load(&r("s")).expect("loads").value;
    let SessionRules::Text(rules) = &loaded.rules else {
        panic!("text")
    };
    assert!(!rules.importance.elements.contains_key("String"));
    // A save of the loaded session writes the reset again (the defaults still hold the element).
    service
        .session_save(&r("s"), &loaded, false)
        .expect("saved");
    assert!(read(&sessions.join("s.toml")).contains("String = \"format\""));
    // The defaults reset: nothing below them holds the element, so their entry goes.
    edit(
        &service,
        &defaults,
        "rules.importance.elements.String",
        "format",
    )
    .expect("reset");
    assert!(!read(&sessions.join("_defaults/text-compare.toml")).contains("String"));
    // The word quoted as a TOML string reads the same.
    edit(
        &service,
        &named,
        "rules.importance.elements.Comment",
        "false",
    )
    .expect("set");
    edit(
        &service,
        &named,
        "rules.importance.elements.Comment",
        "\"format\"",
    )
    .expect("a quoted reset");
    let text = read(&sessions.join("s.toml"));
    assert!(!text.contains("Comment"), "{text}");
    for bad in ["maybe", "\"other\"", "1"] {
        let e = edit(&service, &named, "rules.importance.elements.Comment", bad).unwrap_err();
        assert!(matches!(e, ApiError::InvalidArgument(_)), "{bad}: {e:?}");
        // Core names the words it takes.
        assert!(
            e.to_string().contains("true, false or format"),
            "{bad}: {e}"
        );
    }
    let e = edit(
        &service,
        &named,
        "rules.importance.elements.Everything Else",
        "false",
    )
    .unwrap_err();
    assert!(matches!(e, ApiError::InvalidArgument(_)), "{e:?}");
}

/// The scopes (D-82 with D-317): a reset written to the defaults removes their entry; to a pair override it is
/// `"format"` when the defaults or the type override hold the element, else the override's entry goes.
#[test]
fn a_reset_with_each_scope() {
    use hikasa_session::sessions::{Override, OverrideScope, ParentLink, Session};
    let rules = |entries: &[(&str, bool)]| {
        let mut r = TextRules::default();
        for (k, v) in entries {
            r.importance.elements.insert((*k).to_owned(), *v);
        }
        SessionRules::Text(r)
    };
    let mut defaults = Session::builtin(SessionType::TextCompare);
    defaults.rules = rules(&[("Comment", false)]);
    let mut view = Session::builtin(SessionType::TextCompare);
    view.parent = Some(ParentLink {
        session: "p".into(),
        left: "a.c".into(),
        right: "a.c".into(),
    });
    view.rules = rules(&[("Comment", false), ("String", true)]);
    let change = rules(&[]);
    // Defaults: the entry goes.
    let (mut d, mut v) = (defaults.clone(), view.clone());
    apply_scope(Scope::Defaults, &change, &mut v, None, &mut d).unwrap();
    assert_eq!(d.rules, rules(&[]));
    // Pair: `Comment` is inherited (the defaults), `String` is not.
    let (mut d, mut v) = (defaults.clone(), view.clone());
    let mut parent = Session::builtin(SessionType::FolderCompare);
    let mut own = toml::Table::new();
    own.insert(
        "importance".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "elements".to_owned(),
            toml::Value::Table(toml::Table::from_iter([(
                "String".to_owned(),
                toml::Value::Boolean(true),
            )])),
        )])),
    );
    parent.overrides.push(Override {
        scope: OverrideScope::Pair,
        left: "a.c".into(),
        right: "a.c".into(),
        kind: SessionType::TextCompare,
        rules: own,
    });
    apply_scope(Scope::Pair, &change, &mut v, Some(&mut parent), &mut d).unwrap();
    let text = toml::to_string(&parent.overrides[0].rules).unwrap();
    assert!(text.contains("Comment = \"format\""), "{text}");
    assert!(!text.contains("String"), "{text}");
    let mut child = view.clone();
    child.rules = rules(&[]);
    let SessionRules::Text(r) =
        hikasa_session::sessions::effective_rules(&defaults, Some(&parent), &child)
    else {
        panic!("text")
    };
    assert!(
        r.importance.elements.is_empty(),
        "{:?}",
        r.importance.elements
    );
    // Pair below a type override that holds the element (the defaults do not): the reset is written `"format"`.
    let mut d = Session::builtin(SessionType::TextCompare);
    let mut v = view.clone();
    v.rules = rules(&[("Keyword", false)]);
    let mut parent = Session::builtin(SessionType::FolderCompare);
    parent.overrides.push(Override {
        scope: OverrideScope::Type,
        left: String::new(),
        right: String::new(),
        kind: SessionType::TextCompare,
        rules: toml::from_str("[importance.elements]\nKeyword = false\n").unwrap(),
    });
    apply_scope(Scope::Pair, &change, &mut v, Some(&mut parent), &mut d).unwrap();
    let pair = parent
        .overrides
        .iter()
        .find(|o| o.scope == OverrideScope::Pair)
        .expect("the pair override");
    let text = toml::to_string(&pair.rules).unwrap();
    assert!(text.contains("Keyword = \"format\""), "{text}");
    let mut child = view.clone();
    child.rules = rules(&[]);
    let SessionRules::Text(r) =
        hikasa_session::sessions::effective_rules(&d, Some(&parent), &child)
    else {
        panic!("text")
    };
    assert!(
        !r.importance.elements.contains_key("Keyword"),
        "{:?}",
        r.importance.elements
    );
}

/// Sessions and defaults without `"format"` read and write exactly as before: every session fixture loads and saves
/// back to the same bytes (the goldens of A2.1 are those files).
#[test]
fn files_without_format_entries_are_unchanged() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions");
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let sessions = dir.path().join("config/sessions");
    let mut seen = 0;
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if path.extension().is_none_or(|e| e != "toml") || name.contains("before") {
            continue;
        }
        let text = read(&path);
        write(&sessions.join(format!("{name}.toml")), &text);
        let reference = r(&name);
        let Ok(loaded) = service.session_load(&reference) else {
            continue;
        };
        service
            .session_save(&reference, &loaded.value, true)
            .expect("saved");
        assert_eq!(read(&sessions.join(format!("{name}.toml"))), text, "{name}");
        seen += 1;
    }
    assert!(seen >= 5, "{seen} fixtures");
}
