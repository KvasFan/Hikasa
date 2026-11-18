//! The format editor's core calls (M2 W11c; contract §1.11.27, §6.11 A11.25 core): `format_save` writes what `format
//! edit --set` writes for the same keys (canonical TOML, a built-in's override file); the unknown keys of the file it
//! replaces are kept — the model's unknown keys only, never a setting the user removed (W11c review F1); a file changed
//! since it was read is a conflict; `format_check` names the grammar item; `format_preview`'s tokens equal `format
//! tokens` on the same sample, and on a file loaded as the sample (BOM, UTF-16, a cut at 64 KiB: W11c review F3).

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

/// Saves `id` twice through the editor's calls: first with `first` applied, then with `second` (each on the format as
/// read back), and returns the format read back at the end with the file's text.
fn save_twice(
    id: &str,
    first: impl Fn(&mut FileFormat),
    second: impl Fn(&mut FileFormat),
) -> (FileFormat, String) {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    for step in [&first as &dyn Fn(&mut FileFormat), &second] {
        let read = svc.format_for_edit(id).expect("read");
        let mut format = read.format.clone();
        step(&mut format);
        svc.format_save(id, &format, Some(read.stamp))
            .expect("saved");
    }
    let back = svc.format_for_edit(id).expect("read back").format;
    let text =
        std::fs::read_to_string(dir.path().join(format!("config/formats/{id}.toml"))).unwrap();
    (back, text)
}

/// W11c review F1: a setting the canonical TOML leaves out once removed (no grammar, no element settings, no line
/// weights) is written as removed — the unknown-key merge never brings it back from the file it replaces.
#[test]
fn a_removed_grammar_element_or_line_weight_is_written() {
    let weight = LineWeight {
        pattern: "TODO".into(),
        regex: false,
        match_case: true,
        priority: 2,
    };
    let (back, text) = save_twice(
        "c-cpp",
        |f| {
            assert!(
                !f.grammar.is_empty() && !f.elements.is_empty(),
                "c-cpp has both"
            );
            f.line_weights.push(weight.clone());
        },
        |f| {
            assert_eq!(f.line_weights.len(), 1, "the weight was saved");
            f.grammar.clear();
            f.elements.clear();
            f.line_weights.clear();
        },
    );
    assert!(back.grammar.is_empty(), "grammar came back:\n{text}");
    assert!(back.elements.is_empty(), "elements came back:\n{text}");
    assert!(
        back.line_weights.is_empty(),
        "a line weight came back:\n{text}"
    );
    // One element setting removed, one grammar item removed: the others stay.
    let builtin = service(tempfile::tempdir().unwrap().path())
        .format_for_edit("c-cpp")
        .unwrap()
        .format;
    let first = builtin.elements.keys().next().unwrap().clone();
    let (back, _) = save_twice(
        "c-cpp",
        |f| f.text.tab_size = 8,
        |f| {
            f.elements.remove(&first);
            f.grammar.remove(0);
        },
    );
    assert!(!back.elements.contains_key(&first), "{first:?} came back");
    assert_eq!(back.elements.len(), builtin.elements.len() - 1);
    assert_eq!(back.grammar, builtin.grammar[1..]);
}

/// W11c review F1: every table setting set back to its default after a save with another value is written as the
/// default (the canonical TOML leaves defaults out).
#[test]
fn each_table_setting_back_to_its_default_is_written() {
    use hikasa_formats::{DateOrderSetting, FirstLineSetting, TableSettings, TableType};
    // Each case: a value saved, then the default saved over it.
    type Set = fn(&mut TableSettings);
    let cases: Vec<(&str, Set, Set)> = vec![
        (
            "trim_whitespace",
            |t| t.trim_whitespace = true,
            |t| t.trim_whitespace = false,
        ),
        (
            "ctrl_z_eof",
            |t| t.ctrl_z_eof = true,
            |t| t.ctrl_z_eof = false,
        ),
        (
            "sniff_delimiter",
            |t| t.sniff_delimiter = true,
            |t| t.sniff_delimiter = false,
        ),
        (
            "first_line",
            |t| t.first_line = FirstLineSetting::Names,
            |t| t.first_line = FirstLineSetting::Detect,
        ),
        (
            "delimiters",
            |t| t.delimiters = ";|".into(),
            |t| t.delimiters.clear(),
        ),
        (
            "kind and widths",
            |t| {
                t.kind = TableType::Fixed;
                t.widths = vec![3, 4];
            },
            |t| {
                t.kind = TableType::Delimited;
                t.widths.clear();
            },
        ),
        (
            "regional",
            |t| {
                t.regional.decimal = ",".into();
                t.regional.thousands = ".".into();
                t.regional.date_order = DateOrderSetting::Dmy;
            },
            |t| t.regional = Default::default(),
        ),
    ];
    let preset = service(tempfile::tempdir().unwrap().path())
        .format_for_edit("csv")
        .unwrap()
        .format
        .table
        .unwrap();
    for (name, set, reset) in cases {
        let (back, text) = save_twice(
            "csv",
            |f| set(f.table.as_mut().unwrap()),
            |f| reset(f.table.as_mut().unwrap()),
        );
        let mut wanted = preset.clone();
        reset(&mut wanted);
        assert_eq!(
            back.table.expect("a table"),
            wanted,
            "{name} came back:\n{text}"
        );
    }
}

/// D-70 with the model's schema: the unknown keys of every table are kept (the top level, `text`, an element setting, a
/// grammar item, a line weight, `table.regional` at its defaults), never over a key the format holds; a removed
/// element setting or grammar item takes its unknown keys with it.
#[test]
fn unknown_keys_of_every_table_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let read = svc.format_for_edit("csv").unwrap();
    let mut format = read.format.clone();
    format.line_weights.push(LineWeight {
        pattern: "x".into(),
        regex: false,
        match_case: true,
        priority: 2,
    });
    format.elements.insert(
        hikasa_formats::ElementName::new("Field").unwrap(),
        hikasa_formats::ElementInfo {
            case_sensitive: false,
            important: true,
        },
    );
    svc.format_save("csv", &format, Some(read.stamp)).unwrap();
    let path = dir.path().join("config/formats/csv.toml");
    let mut doc: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let table = |v: &mut toml::Value| v.as_table_mut().unwrap().clone();
    doc.insert("future".into(), 1.into());
    let mut text = table(doc.get_mut("text").unwrap());
    text.insert("wrap".into(), true.into());
    doc.insert("text".into(), text.into());
    let mut t = table(doc.get_mut("table").unwrap());
    let mut regional = toml::Table::new();
    regional.insert("era".into(), "x".into());
    t.insert("regional".into(), regional.into());
    t.insert("bold".into(), false.into());
    doc.insert("table".into(), t.into());
    let weights = doc.get_mut("line_weight").unwrap().as_array_mut().unwrap();
    weights[0]
        .as_table_mut()
        .unwrap()
        .insert("weight".into(), 3.into());
    let elements = doc.get_mut("elements").unwrap().as_table_mut().unwrap();
    elements
        .get_mut("Field")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("bold".into(), true.into());
    std::fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
    let keys = |text: &str| {
        let mut keys = hikasa_formats::unknown_keys(&text.parse::<toml::Table>().unwrap());
        keys.sort();
        keys
    };
    let all = [
        "elements.Field.bold",
        "future",
        "line_weight[0].weight",
        "table.bold",
        "table.regional.era",
        "text.wrap",
    ];
    assert_eq!(keys(&std::fs::read_to_string(&path).unwrap()), all);
    // A description edit keeps every unknown key.
    let read = svc.format_for_edit("csv").unwrap();
    let mut format = read.format.clone();
    format.description = "edited".into();
    svc.format_save("csv", &format, Some(read.stamp)).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(keys(&saved), all, "{saved}");
    let back = svc.format_for_edit("csv").unwrap().format;
    assert_eq!(back.description, "edited");
    // The element setting and the line weight removed: their unknown keys go with them.
    let read = svc.format_for_edit("csv").unwrap();
    let mut format = read.format.clone();
    format.elements.clear();
    format.line_weights.clear();
    svc.format_save("csv", &format, Some(read.stamp)).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        keys(&saved),
        ["future", "table.bold", "table.regional.era", "text.wrap"],
        "{saved}"
    );
    let back = svc.format_for_edit("csv").unwrap().format;
    assert!(
        back.elements.is_empty() && back.line_weights.is_empty(),
        "{saved}"
    );
    assert_eq!(back.table.unwrap().regional, Default::default());
    // An unknown key never replaces a key the format writes: `label` is unknown in auto mode, known in fixed mode.
    let doc = format!("{saved}\n");
    let mut doc: toml::Table = doc.parse().unwrap();
    doc.get_mut("encoding")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("label".into(), "latin1".into());
    std::fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
    let read = svc.format_for_edit("csv").unwrap();
    let mut format = read.format.clone();
    format.encoding = hikasa_formats::EncodingSetting::Fixed {
        label: "utf-16le".into(),
    };
    svc.format_save("csv", &format, Some(read.stamp)).unwrap();
    let back = svc.format_for_edit("csv").unwrap().format;
    assert_eq!(back.encoding, format.encoding);
}

/// The tokens `format tokens` lists for `file` with c-cpp.
fn tokens_of(svc: &Service, file: &Path) -> Vec<hikasa_session::api::format_editor::PreviewLine> {
    svc.format_tokens(
        &FormatTokensRequest {
            file: file.as_os_str().to_owned(),
            format: Some("c-cpp".into()),
            encoding: None,
            lines: None,
        },
        &job(svc, JobKind::FormatTokens),
    )
    .expect("tokens")
    .lines
}

/// The preview of `sample` with c-cpp.
fn preview_of(svc: &Service, sample: &str) -> Vec<hikasa_session::api::format_editor::PreviewLine> {
    let format = svc.format_for_edit("c-cpp").unwrap().format;
    svc.format_preview(&format, sample, &job(svc, JobKind::FormatTokens))
        .expect("preview")
}

/// W11c review F3: a file loaded as the sample is decoded as `format tokens` loads it — a BOM left out, UTF-16 and
/// UTF-32 decoded — so its preview equals `format tokens` of the file.
#[test]
fn the_preview_of_a_loaded_file_equals_format_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let c_cpp = svc.format_for_edit("c-cpp").unwrap().format;
    let text = "int x; // c\n/* y */ \"é\"\n";
    let utf16 = |big: bool| {
        let mut bytes = if big {
            vec![0xFE, 0xFF]
        } else {
            vec![0xFF, 0xFE]
        };
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&if big {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        bytes
    };
    let mut utf32 = vec![0xFF, 0xFE, 0, 0];
    for c in text.chars() {
        utf32.extend_from_slice(&u32::from(c).to_le_bytes());
    }
    let bom8 = [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat();
    for (name, bytes) in [
        ("utf-8 BOM", bom8),
        ("utf-16le", utf16(false)),
        ("utf-16be", utf16(true)),
        ("utf-32le", utf32),
    ] {
        let file = write(dir.path(), "s.c", &bytes);
        let sample = svc.format_sample(&file, &c_cpp).expect("sample");
        assert_eq!(sample, text, "{name}");
        assert_eq!(preview_of(&svc, &sample), tokens_of(&svc, &file), "{name}");
    }
    // The format's fixed encoding decodes the sample, as it decodes the document.
    let mut latin = c_cpp.clone();
    latin.encoding = hikasa_formats::EncodingSetting::Fixed {
        label: "windows-1252".into(),
    };
    let file = write(dir.path(), "l.c", b"int \xE9;\n");
    assert_eq!(svc.format_sample(&file, &latin).unwrap(), "int é;\n");
    // A Ctrl-Z ends the text of a format with the Ctrl-Z rule.
    let mut ctrl_z = c_cpp.clone();
    ctrl_z.text.ctrl_z_eof = true;
    let file = write(dir.path(), "z.c", b"int a;\n\x1Atail\n");
    assert_eq!(svc.format_sample(&file, &ctrl_z).unwrap(), "int a;\n");
    assert_eq!(
        svc.format_sample(&file, &c_cpp).unwrap(),
        "int a;\n\x1Atail\n"
    );
}

/// W11c review F3: the sample holds the first 64 KiB cut at the last whole character (no U+FFFD for a character the
/// bound cuts), and its preview equals `format tokens` on every whole line; a file of exactly 64 KiB is whole.
#[test]
fn the_sample_is_cut_at_a_whole_character_within_64_kib() {
    use hikasa_session::api::format_editor::PREVIEW_SAMPLE_BYTES;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let c_cpp = svc.format_for_edit("c-cpp").unwrap().format;
    let line = "int x; /* é */\n";
    let mut text = line.repeat(PREVIEW_SAMPLE_BYTES / line.len() - 1);
    while text.len() < PREVIEW_SAMPLE_BYTES - 1 {
        text.push('a');
    }
    // 'é' takes the last byte of the bound and the first one past it.
    text.push_str("é more\n");
    let file = write(dir.path(), "big.c", text.as_bytes());
    let sample = svc.format_sample(&file, &c_cpp).unwrap();
    assert!(
        !sample.contains('\u{FFFD}'),
        "a replacement character at the cut"
    );
    assert_eq!(sample.len(), PREVIEW_SAMPLE_BYTES - 1);
    assert!(text.starts_with(&sample));
    let preview = preview_of(&svc, &sample);
    let tokens = tokens_of(&svc, &file);
    let whole = preview.len() - 1;
    assert_eq!(preview[..whole], tokens[..whole]);
    // UTF-16: a surrogate pair cut by the bound is left out too.
    let mut bytes = vec![0xFF, 0xFE];
    while bytes.len() < PREVIEW_SAMPLE_BYTES - 2 {
        bytes.extend_from_slice(&u16::from(b'a').to_le_bytes());
    }
    for unit in "😀z".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let file = write(dir.path(), "wide.c", &bytes);
    let sample = svc.format_sample(&file, &c_cpp).unwrap();
    assert!(
        !sample.contains('\u{FFFD}'),
        "a replacement character at the cut"
    );
    assert_eq!(sample.len(), (PREVIEW_SAMPLE_BYTES - 4) / 2);
    // Exactly the bound: nothing is cut, the last byte stays one character (as the document reads it).
    let mut exact = vec![b'a'; PREVIEW_SAMPLE_BYTES - 1];
    exact.push(0xC3);
    let file = write(dir.path(), "exact.c", &exact);
    let sample = svc.format_sample(&file, &c_cpp).unwrap();
    assert_eq!(sample.chars().count(), PREVIEW_SAMPLE_BYTES);
    // A file over the bound: never more than the bound.
    let file = write(dir.path(), "huge.c", &vec![b'b'; 3 * PREVIEW_SAMPLE_BYTES]);
    assert_eq!(
        svc.format_sample(&file, &c_cpp).unwrap().len(),
        PREVIEW_SAMPLE_BYTES
    );
}

/// A format is saved under its own id only: another id's file is refused, nothing written.
#[test]
fn a_format_is_saved_under_its_own_id_only() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let c_cpp = svc.format_for_edit("c-cpp").unwrap().format;
    let e = svc.format_save("java", &c_cpp, None).unwrap_err();
    assert!(matches!(e, ApiError::InvalidArgument(_)), "{e:?}");
    assert_eq!(e.to_string(), "the format id is `c-cpp`, not `java`");
    assert!(formats_dir(dir.path()).is_empty(), "nothing written");
}
