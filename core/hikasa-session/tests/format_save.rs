//! The format editor's core calls (M2 W11c; contract §1.11.27, §6.11 A11.25 core): `format_save` writes what `format
//! edit --set` writes for the same keys (canonical TOML, a built-in's override file); the unknown keys of the file it
//! replaces are kept; a file changed since it was read is a conflict; `format_check` names the grammar item;
//! `format_preview`'s tokens equal `format tokens` on the same sample.

mod common;

use std::path::Path;

use common::{job, service, write};
use hikasa_core::JobKind;
use hikasa_formats::{FileFormat, GrammarItem, ItemKind, LineWeight};
use hikasa_session::Service;
use hikasa_session::api::{ApiError, FormatTokensRequest};
use hikasa_session::settings::ChangeStamp;

/// Two configurations: the editor's and its twin for the CLI's command.
fn twins() -> (tempfile::TempDir, Service, tempfile::TempDir, Service) {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (sa, sb) = (service(a.path()), service(b.path()));
    (a, sa, b, sb)
}

fn formats_dir(root: &Path) -> std::collections::BTreeMap<String, String> {
    let dir = root.join("config/formats");
    let mut out = std::collections::BTreeMap::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for e in read.flatten() {
            out.insert(
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap_or_default(),
            );
        }
    }
    out
}

/// One edit: the keys `format edit --set` takes, and the same change on the model.
type Edit = (
    &'static str,
    Vec<(&'static str, &'static str)>,
    fn(&mut FileFormat),
);

fn edits() -> Vec<Edit> {
    vec![
        ("c-cpp", vec![("mask", "\"*.c;*.h\"")], |f| {
            f.mask = "*.c;*.h".into()
        }),
        ("c-cpp", vec![("text.tab_size", "8")], |f| {
            f.text.tab_size = 8
        }),
        ("c-cpp", vec![("description", "\"edited\"")], |f| {
            f.description = "edited".into()
        }),
        ("c-cpp", vec![("conversion.method", "\"sorted\"")], |f| {
            f.conversion = hikasa_formats::Conversion::Sorted
        }),
        ("c-cpp", vec![("text.save_trim_trailing_ws", "true")], |f| {
            f.text.save_trim_trailing_ws = true
        }),
        ("c-cpp", vec![("grammar.1.stop_at_eol", "true")], |f| {
            if let Some(GrammarItem {
                kind: ItemKind::Delimited { stop_at_eol, .. },
                ..
            }) = f.grammar.get_mut(1)
            {
                *stop_at_eol = true;
            }
        }),
    ]
}

#[test]
fn format_save_equals_format_edit_set() {
    for (id, keys, change) in edits() {
        let (a, sa, b, sb) = twins();
        let read = sa.format_for_edit(id).expect("read");
        assert_eq!(read.origin, "built_in");
        assert_eq!(read.stamp, ChangeStamp::Absent);
        let mut format = read.format.clone();
        change(&mut format);
        sa.format_save(id, &format, Some(read.stamp))
            .expect("saved");
        let set: Vec<(String, String)> = keys
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        sb.format_edit(id, &set, None).expect("format edit");
        assert_eq!(formats_dir(a.path()), formats_dir(b.path()), "{keys:?}");
        assert_eq!(sa.format_for_edit(id).unwrap().origin, "override");
    }
}

#[test]
fn unknown_keys_are_kept_and_a_conflict_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    svc.format_new("mine", None, Some("Mine"), Some("*.mine"), None)
        .expect("new");
    let path = dir.path().join("config/formats/mine.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\nfuture_key = 3\n")).unwrap();
    let svc = service(dir.path());
    let read = svc.format_for_edit("mine").expect("read");
    assert_eq!(read.origin, "user");
    let mut format = read.format.clone();
    format.description = "described".into();
    svc.format_save("mine", &format, Some(read.stamp))
        .expect("saved");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("future_key = 3"), "{saved}");
    assert!(saved.contains("described"), "{saved}");
    // Changed on disk since it was read: a conflict, nothing written.
    let stale = svc.format_for_edit("mine").unwrap().stamp;
    std::fs::write(&path, format!("{saved}\n# touched\n")).unwrap();
    format.description = "again".into();
    let e = svc.format_save("mine", &format, Some(stale)).unwrap_err();
    assert!(
        matches!(
            &e,
            ApiError::Session(hikasa_session::SessionError::Conflict { .. })
        ),
        "{e:?}"
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("# touched")
    );
    // Overwrite: no stamp.
    svc.format_save("mine", &format, None).expect("overwrite");
    assert!(std::fs::read_to_string(&path).unwrap().contains("again"));
}

#[test]
fn format_check_names_the_item() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut format = svc.format_for_edit("c-cpp").unwrap().format;
    assert!(svc.format_check(&format).expect("checks").is_empty());
    format.grammar.insert(
        2,
        GrammarItem {
            element: hikasa_formats::ElementName::new("Bad").unwrap(),
            kind: ItemKind::Basic {
                text: "(unclosed".into(),
                regex: true,
                match_case: true,
            },
        },
    );
    let e = svc.format_check(&format).unwrap_err().to_string();
    assert!(e.starts_with("Grammar item 3"), "{e}");
    // An item its TOML cannot hold (Lines without a heading and without *or line 1*: the heading is not written) is
    // named by the item check before the format is read back.
    let mut format = svc.format_for_edit("c-cpp").unwrap().format;
    format.grammar[4].kind = ItemKind::Lines {
        heading: String::new(),
        regex: false,
        match_case: true,
        next_lines: 2,
        or_line_one: false,
    };
    let e = svc.format_check(&format).unwrap_err().to_string();
    assert!(
        e.starts_with("Grammar item 5") && e.contains("heading"),
        "{e}"
    );
    let mut format = svc.format_for_edit("c-cpp").unwrap().format;
    format.grammar[0].kind = ItemKind::Columns {
        from_column: 5,
        to_column: Some(2),
    };
    let e = svc.format_check(&format).unwrap_err().to_string();
    assert!(e.starts_with("Grammar item 1"), "{e}");
    let mut format = svc.format_for_edit("c-cpp").unwrap().format;
    format.line_weights.push(LineWeight {
        pattern: "[".into(),
        regex: true,
        match_case: true,
        priority: 2,
    });
    let e = svc.format_check(&format).unwrap_err().to_string();
    assert!(e.starts_with("Line weight "), "{e}");
    let e = svc
        .format_save("c-cpp", &format, None)
        .unwrap_err()
        .to_string();
    assert!(e.starts_with("Line weight "), "{e}");
}

#[test]
fn format_preview_equals_format_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let sample = "/* a\n   comment */ int x = 1; // end\r\n\"str\" return;\n";
    let file = write(dir.path(), "s.c", sample.as_bytes());
    let format = svc.format_for_edit("c-cpp").unwrap().format;
    let preview = svc
        .format_preview(&format, sample, &job(&svc, JobKind::FormatTokens))
        .expect("preview");
    let tokens = svc
        .format_tokens(
            &FormatTokensRequest {
                file: file.into_os_string(),
                format: Some("c-cpp".into()),
                encoding: Some("utf-8".into()),
                lines: None,
            },
            &job(&svc, JobKind::FormatTokens),
        )
        .expect("tokens");
    assert_eq!(preview, tokens.lines);
    assert!(
        preview
            .iter()
            .any(|l| l.tokens.iter().any(|t| t.element == "Comment"))
    );
    // A format that does not compile: core's message.
    let mut bad = format.clone();
    bad.line_weights.push(LineWeight {
        pattern: "(".into(),
        regex: true,
        match_case: true,
        priority: 1,
    });
    assert!(
        svc.format_preview(&bad, sample, &job(&svc, JobKind::FormatTokens))
            .is_err()
    );
}

#[test]
fn the_regex_helper_items_compile() {
    let limits = hikasa_formats::RegexLimits::default();
    for item in hikasa_formats::regex_helper::ITEMS {
        let source = match item.insert {
            "*" | "+" | "?" => format!("a{}", item.insert),
            "[ ]" | "[^ ]" => item.insert.replace(' ', "x"),
            other => other.to_owned(),
        };
        hikasa_formats::Pattern::compile(&source, false, &limits)
            .unwrap_or_else(|e| panic!("{}: {e}", item.caption));
    }
}
