//! The built-in presets (FF-040, Appendix A; §6.2.4): every preset parses without warnings, compiles,
//! round-trips through `to_toml` (INV-FMT-1), starts with a `# Sources:` comment and carries its keyword list;
//! preset order and enabled flags (D-26, D-27); element ids (D-41); detection and shadowing of the built-ins.

use std::sync::Arc;

use hikasa_formats::{
    CompiledFormat, DetectReason, ElementId, EnabledOverrides, FormatId, FormatKind,
    FormatRegistry, ItemKind, PRESETS, RegexLimits, ShadowStatus, parse_format, to_toml,
};

#[test]
fn every_preset_parses_compiles_and_round_trips() {
    for preset in PRESETS {
        assert!(
            preset.toml.starts_with("# Sources: "),
            "{} must start with a # Sources: comment",
            preset.file
        );
        let (format, warnings) = parse_format(preset.toml, &preset.origin()).unwrap();
        assert!(warnings.is_empty(), "{}: {warnings:?}", preset.file);
        let stem = preset.file.trim_end_matches(".toml");
        assert_eq!(&stem[3..], format.id.as_str(), "file name and id");
        let compiled = CompiledFormat::compile(Arc::new(format.clone()), &RegexLimits::default())
            .unwrap_or_else(|e| panic!("{}: {e}", preset.file));
        assert_eq!(compiled.id(), Some(&format.id));
        let canonical = to_toml(&format);
        let (again, warnings) = parse_format(&canonical, "canonical").unwrap();
        assert!(warnings.is_empty(), "{}: {warnings:?}", preset.file);
        assert_eq!(again, format, "{} round trip:\n{canonical}", preset.file);
        assert_eq!(to_toml(&again), canonical);
    }
}

/// Preset order (= file prefix) and enabled flags (D-27); the CSV mask (D-26).
#[test]
fn preset_order_and_enabled_flags() {
    let files: Vec<_> = PRESETS.iter().map(|p| p.file).collect();
    assert_eq!(files.len(), 21);
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted, "PRESETS order is the file prefix order");
    for (k, file) in files.iter().enumerate() {
        assert!(file.starts_with(&format!("{:02}-", k + 1)), "{file}");
    }
    let registry = FormatRegistry::builtin().unwrap();
    let ids: Vec<_> = registry
        .entries()
        .iter()
        .map(|e| (e.id().as_str(), e.enabled))
        .collect();
    assert_eq!(
        ids,
        [
            ("bash", true),
            ("c-cpp", true),
            ("cobol", true),
            ("pascal-forms", true),
            ("pascal", true),
            ("html", true),
            ("java", true),
            ("javascript", true),
            ("perl", true),
            ("python", true),
            ("ini-reg", true),
            ("sql", true),
            ("visual-basic", true),
            ("xml", true),
            ("rust", true),
            ("sorted", true),
            ("csv", true),
            ("pictures", true),
            ("media", true),
            ("executables", true),
            ("registry-files", false),
        ]
    );
    let csv = registry.get(&FormatId::new("csv").unwrap()).unwrap();
    assert_eq!(csv.format.mask, "*.csv;*.tsv");
    assert_eq!(csv.format.kind, FormatKind::Table);
    let names: Vec<_> = registry
        .entries()
        .iter()
        .map(|e| e.format.name.as_str())
        .collect();
    assert_eq!(names[1], "C, C++, C#, ObjC Source");
    assert_eq!(names[10], "Registry Dumps / INI");
    // Fresh value on each call (no global cache).
    let again = FormatRegistry::builtin().unwrap();
    assert_eq!(again.entries().len(), registry.entries().len());
}

/// Every preset for which Appendix A.3 gives a keyword list has a non-empty `Keyword` List item. HTML finds its
/// keywords by position instead (tag names after `<`/`</`, attribute names before `=`; Appendix A.2), so that
/// words in running text are never keywords.
#[test]
fn keyword_lists() {
    let registry = FormatRegistry::builtin().unwrap();
    let with_lists = [
        ("bash", 75),
        ("c-cpp", 197),
        ("cobol", 185),
        ("pascal-forms", 7),
        ("pascal", 110),
        ("java", 104),
        ("javascript", 75),
        ("perl", 82),
        ("python", 39),
        ("ini-reg", 3),
        ("sql", 113),
        ("visual-basic", 150),
        ("rust", 54),
    ];
    for (id, at_least) in with_lists {
        let entry = registry.get(&FormatId::new(id).unwrap()).unwrap();
        let tokens: usize = entry
            .format
            .grammar
            .iter()
            .filter(|g| g.element.as_str() == "Keyword")
            .map(|g| match &g.kind {
                ItemKind::List { tokens, .. } => tokens.len(),
                _ => 0,
            })
            .sum();
        assert!(tokens >= at_least, "{id}: {tokens} keywords");
    }
    let html = registry.get(&FormatId::new("html").unwrap()).unwrap();
    let keyword_patterns: Vec<&str> = html
        .format
        .grammar
        .iter()
        .filter(|g| g.element.as_str() == "Keyword")
        .filter_map(|g| match &g.kind {
            ItemKind::Basic {
                text, regex: true, ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        keyword_patterns,
        [r"(?<=<|</)[A-Za-z][\w.:-]*", r"[A-Za-z_:][\w.:-]*(?=\s*=)"]
    );
    for id in [
        "html",
        "xml",
        "sorted",
        "csv",
        "pictures",
        "media",
        "executables",
        "registry-files",
    ] {
        let entry = registry.get(&FormatId::new(id).unwrap()).unwrap();
        assert!(
            !entry
                .format
                .grammar
                .iter()
                .any(|g| matches!(g.kind, ItemKind::List { .. })),
            "{id}"
        );
    }
}

/// Element ids of the C-family preset (Appendix A.1) and the unimportant elements (D-04).
#[test]
fn element_ids_and_importance() {
    let registry = FormatRegistry::builtin().unwrap();
    let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    let names: Vec<_> = c.grammar().elements().iter().map(|(_, n)| n).collect();
    assert_eq!(
        names,
        [
            "Everything Else",
            "Comment",
            "String",
            "Directive",
            "Keyword",
            "Identifier",
            "Number",
            "Operator"
        ]
    );
    assert!(!c.grammar().elements().case_sensitive(ElementId(6)));
    assert!(c.grammar().has_multiline());
    let py = registry
        .compiled(&FormatId::new("python").unwrap())
        .unwrap();
    let ws = py.grammar().elements().id("Whitespace").unwrap();
    assert!(!py.grammar().elements().default_important(ws));
    let indent = py.grammar().elements().id("Indent").unwrap();
    assert!(py.grammar().elements().default_important(indent));
    let cobol = registry.compiled(&FormatId::new("cobol").unwrap()).unwrap();
    let ident = cobol.grammar().elements().id("Identification").unwrap();
    assert!(!cobol.grammar().elements().default_important(ident));
    assert_eq!(cobol.text().word_chars, "-");
    for entry in registry.entries() {
        let compiled = registry.compiled(entry.id()).unwrap();
        for (id, name) in compiled.grammar().elements().iter() {
            let expected = !matches!(name, "Whitespace" | "Identification");
            assert_eq!(
                compiled.grammar().elements().default_important(id),
                expected,
                "{} {name}",
                entry.id()
            );
        }
    }
    let forms = registry
        .compiled(&FormatId::new("pascal-forms").unwrap())
        .unwrap();
    assert_eq!(forms.line_weight("  object Button1: TButton"), 4);
    assert_eq!(forms.line_weight("  Caption = 'x'"), 0);
    let pascal = registry
        .compiled(&FormatId::new("pascal").unwrap())
        .unwrap();
    assert_eq!(pascal.line_weight("  TForm1 = CLASS(TForm)"), 4);
}

/// Detection with the built-ins (FF-001, D-10, D-26, D-27).
#[test]
fn builtin_detection() {
    let registry = FormatRegistry::builtin().unwrap();
    let none = EnabledOverrides::default();
    let cases = [
        ("main.c", Some("c-cpp")),
        ("MAIN.C", Some("c-cpp")),
        ("Widget.HPP", Some("c-cpp")),
        ("build.sh", Some("bash")),
        ("PAYROLL.CBL", Some("cobol")),
        ("Form1.dfm", Some("pascal-forms")),
        ("unit1.pas", Some("pascal")),
        ("index.html", Some("html")),
        ("Main.kt", Some("java")),
        ("app.tsx", Some("javascript")),
        ("tool.pm", Some("perl")),
        ("setup.py", Some("python")),
        ("settings.reg", Some("ini-reg")),
        ("win.INI", Some("ini-reg")),
        ("query.sql", Some("sql")),
        ("Module1.bas", Some("visual-basic")),
        ("icon.svg", Some("xml")),
        ("lib.rs", Some("rust")),
        ("data.csv", Some("csv")),
        ("data.tsv", Some("csv")),
        ("photo.JPEG", Some("pictures")),
        ("song.flac", Some("media")),
        ("libz.so", Some("executables")),
        ("notes.txt", None),
        ("notes.txt2", None),
        ("Makefile", None),
    ];
    for (name, expected) in cases {
        let d = registry.detect(name, &none).unwrap();
        match expected {
            Some(id) => match d.reason {
                DetectReason::Mask { id: got, .. } => assert_eq!(got.as_str(), id, "{name}"),
                other => panic!("{name}: {other:?}"),
            },
            None => assert_eq!(d.reason, DetectReason::Fallback, "{name}"),
        }
    }
    let sorted = registry.find("sorted").unwrap();
    assert!(sorted.format.mask.is_empty());
}

/// Shadowing of the built-ins: none is red on first run (D-27); enabling "Registry Files" shows it shadowed
/// by "Registry Dumps / INI".
#[test]
fn builtin_shadow_report() {
    let registry = FormatRegistry::builtin().unwrap();
    let report = registry.shadow_report(&EnabledOverrides::default());
    for (id, status) in &report {
        let expected = match id.as_str() {
            "sorted" => ShadowStatus::ManualOnly,
            "registry-files" => ShadowStatus::Disabled,
            _ => ShadowStatus::Visible,
        };
        assert_eq!(*status, expected, "{id}");
    }
    let on = EnabledOverrides(
        [(FormatId::new("registry-files").unwrap(), true)]
            .into_iter()
            .collect(),
    );
    let report = registry.shadow_report(&on);
    assert_eq!(
        report.last().unwrap().1,
        ShadowStatus::Shadowed {
            by: vec![FormatId::new("ini-reg").unwrap()]
        }
    );
}

/// `(element, text)` of every token of one line of a built-in preset.
fn tokens(id: &str, line: &str) -> Vec<(String, String)> {
    let registry = FormatRegistry::builtin().unwrap();
    let c = registry.compiled(&FormatId::new(id).unwrap()).unwrap();
    let g = c.grammar();
    let mut t = hikasa_formats::Tokenizer::new(g);
    let mut out = Vec::new();
    t.tokenize_line(line, 0, hikasa_formats::LineState::Normal, &mut out);
    out.iter()
        .map(|t| {
            (
                g.elements().name(t.element).to_owned(),
                t.text(line).to_owned(),
            )
        })
        .collect()
}

fn has(tokens: &[(String, String)], element: &str, text: &str) -> bool {
    tokens.iter().any(|(e, t)| e == element && t == text)
}

/// COBOL columns 73–80 are always the unimportant Identification element, even when a literal or an
/// identifier would run past column 72, and a comment never includes the padding before column 73
/// (§2.13.3 row 16, Appendix A.2; wave-2 review).
#[test]
fn cobol_identification_area_is_a_field() {
    let line = |body: &str, ident: &str| format!("{body:<72}{ident}");
    let literal = line(
        "000300     DISPLAY \"THIS IS A VERY LONG LITERAL THAT CONTINUES ON TH",
        "PROG0003",
    );
    let t = tokens("cobol", &literal);
    assert!(has(&t, "Identification", "PROG0003"), "{t:?}");
    assert!(
        has(
            &t,
            "String",
            "\"THIS IS A VERY LONG LITERAL THAT CONTINUES ON TH"
        ),
        "{t:?}"
    );
    let identifier = line(
        "000200     MOVE WS-ALPHA-VALUE TO WS-BETA-VALUE-WITH-A-LONG-NAME-XYZABCD",
        "PROG0002",
    );
    let t = tokens("cobol", &identifier);
    assert!(has(&t, "Identification", "PROG0002"), "{t:?}");
    assert!(
        has(&t, "Identifier", "WS-BETA-VALUE-WITH-A-LONG-NAME-XYZABCD"),
        "{t:?}"
    );
    let padded = tokens("cobol", &line("000500*    A COMMENT LINE", "PROG0005"));
    let trimmed = tokens("cobol", "000500*    A COMMENT LINE");
    assert!(has(&padded, "Comment", "*    A COMMENT LINE"), "{padded:?}");
    assert!(
        has(&trimmed, "Comment", "*    A COMMENT LINE"),
        "{trimmed:?}"
    );
    assert!(has(&padded, "Identification", "PROG0005"));
}

/// Python: trailing blanks after a comment are the unimportant `Whitespace` element, as after code
/// (Appendix A.2; wave-2 review).
#[test]
fn python_trailing_whitespace_after_a_comment() {
    let after_code = tokens("python", "x = 1   ");
    let after_comment = tokens("python", "x = 1  # note   ");
    assert_eq!(
        after_code.last().unwrap(),
        &("Whitespace".to_owned(), "   ".to_owned())
    );
    assert_eq!(
        after_comment.last().unwrap(),
        &("Whitespace".to_owned(), "   ".to_owned())
    );
    assert!(has(&after_comment, "Comment", "# note"));
    assert_eq!(
        tokens("python", "#"),
        [("Comment".to_owned(), "#".to_owned())]
    );
}

/// HTML: words in running text are never (case-insensitive) keywords; tag names after `<`/`</` and attribute
/// names before `=` are. HTML and XML attribute values start right after `=` (Appendix A.2; wave-2 review).
#[test]
fn markup_keywords_and_attribute_values() {
    let t = tokens("html", "<p>The Table shows a Form for Time.</p>");
    let keywords: Vec<_> = t.iter().filter(|(e, _)| e == "Keyword").collect();
    assert_eq!(keywords.len(), 2, "{t:?}");
    assert!(has(
        &t,
        "Everything Else",
        "The Table shows a Form for Time."
    ));
    let t = tokens(
        "html",
        "<my-widget data-x=\"1\" aria-label = 'it'>Don't</my-widget>",
    );
    for (element, text) in [
        ("Keyword", "my-widget"),
        ("Keyword", "data-x"),
        ("Keyword", "aria-label"),
        ("String", "\"1\""),
        ("String", "'it'"),
    ] {
        assert!(has(&t, element, text), "{element} {text}: {t:?}");
    }
    assert!(!t.iter().any(|(e, x)| e == "String" && x.contains("Don")));
    // A value that continues on the next line is not a string; the next attributes still pair correctly.
    let t = tokens("xml", "         L 10 10\" fill=\"red\" stroke=\"blue\"/>");
    let strings: Vec<_> = t
        .iter()
        .filter(|(e, _)| e == "String")
        .map(|(_, x)| x.as_str())
        .collect();
    assert_eq!(strings, ["\"red\"", "\"blue\""]);
}
