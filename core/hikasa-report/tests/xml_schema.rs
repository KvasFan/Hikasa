//! M2 W7 acceptance A7.7 (D-199): the XML dumps against their schema. Everywhere: the `roxmltree` structure checks of
//! `common::xmlcheck` on every XML golden and on 64 generated documents per run. On Linux and macOS CI
//! (`HIKASA_REQUIRE_XMLLINT=1`, set in `ci.yml`): `xmllint --noout --schema docs/schemas/hikasa-report-1.xsd` on the same
//! documents; where `xmllint` is missing and not required, and on Windows, that part is skipped with one line on stderr
//! (the bytes are the same on every platform: the goldens prove it there). Negative cases prove the checks can fail: a
//! golden without a required attribute and one with an unknown attribute are rejected by `xmllint` and by the
//! structure checks, and a broken copy of the schema fails.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use common::inputs::{
    HOSTILE, compare_memory, compare_strings, folder_tree, materialize, text_pair,
};
use common::{fixtures, xmlcheck};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, FolderRules};
use hikasa_report::{
    HtmlStyle, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest, ReportWords,
    render_report,
};
use hikasa_text::TextRules;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// The schema.
fn xsd() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/hikasa-report-1.xsd")
}

/// Whether `xmllint` runs here: required where `HIKASA_REQUIRE_XMLLINT=1`, else skipped with one stderr line when
/// missing; always skipped on Windows (§1.7.7).
fn xmllint_available() -> bool {
    let required = std::env::var("HIKASA_REQUIRE_XMLLINT").is_ok_and(|v| v == "1");
    if cfg!(windows) {
        assert!(
            !required,
            "HIKASA_REQUIRE_XMLLINT=1 on Windows, where the xmllint check does not run"
        );
        eprintln!(
            "xml_schema: the xmllint check is skipped on Windows (the goldens prove the same bytes)"
        );
        return false;
    }
    let found = Command::new("xmllint")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        assert!(!required, "HIKASA_REQUIRE_XMLLINT=1 and no xmllint on PATH");
        eprintln!(
            "xml_schema: xmllint not found, the schema check is skipped (set HIKASA_REQUIRE_XMLLINT=1 to require it)"
        );
    }
    found
}

/// Runs `xmllint --noout --schema <schema> <files>`: `Ok` when every file validates, else the tool's messages.
fn xmllint(schema: &Path, files: &[PathBuf]) -> Result<(), String> {
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(schema)
        .args(files)
        .output()
        .expect("xmllint runs");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Every XML golden of the crate.
fn goldens() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![fixtures("")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "xml") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn render(
    input: ReportInput<'_>,
    kind: ReportKind,
    words: &ReportWords,
    defaults: &ReportDefaults,
) -> Vec<u8> {
    let req = ReportRequest::parse(kind, words, defaults).unwrap();
    let meta = match (input, &req.body) {
        (ReportInput::Text(cmp), hikasa_report::ReportBody::Text(opts)) => {
            ReportMeta::for_text(cmp, opts, None, "hikasa test")
        }
        (ReportInput::Folder(cmp), hikasa_report::ReportBody::Folder { options, .. }) => {
            ReportMeta::for_folder(cmp, options, None, "hikasa test")
        }
        _ => unreachable!(),
    };
    let mut meta = meta;
    meta.title = req.title.clone();
    let mut out = Vec::new();
    render_report(input, &req, &meta, &HtmlStyle::default(), None, &mut out).unwrap();
    out
}

/// 32 text and 32 folder documents from random inputs, each checked by the structure checks.
fn generated() -> Vec<Vec<u8>> {
    let docs = Mutex::new(Vec::new());
    let mut runner = TestRunner::new(Config {
        cases: 32,
        failure_persistence: None,
        ..Config::default()
    });
    let displays = [
        "display-all",
        "display-mismatches",
        "display-context",
        "display-none",
        "display-matches",
    ];
    runner
        .run(
            &(
                text_pair(HOSTILE),
                0..displays.len(),
                any::<bool>(),
                proptest::option::of(super_line()),
            ),
            |((left, right), display, iu, title)| {
                let cmp = compare_strings(&left, &right, &TextRules::default());
                let mut options = vec![displays[display].to_owned()];
                if iu {
                    options.push("ignore-unimportant".into());
                }
                let words = ReportWords {
                    layout: Some("xml".into()),
                    options,
                    output_options: Vec::new(),
                    title,
                };
                let doc = render(
                    ReportInput::Text(&cmp),
                    ReportKind::Text,
                    &words,
                    &ReportDefaults::new("xml"),
                );
                xmlcheck::check(&doc).map_err(|e| {
                    TestCaseError::fail(format!("{e}\n{}", String::from_utf8_lossy(&doc)))
                })?;
                docs.lock().unwrap().push(doc);
                Ok(())
            },
        )
        .unwrap();
    let mut runner = TestRunner::new(Config {
        cases: 32,
        failure_persistence: None,
        ..Config::default()
    });
    let filters = [
        FolderDisplayFilter::All,
        FolderDisplayFilter::Differences,
        FolderDisplayFilter::Orphans,
        FolderDisplayFilter::Same,
        FolderDisplayFilter::None,
    ];
    runner
        .run(
            &(
                folder_tree(),
                0..filters.len(),
                any::<bool>(),
                any::<bool>(),
            ),
            |(tree, filter, iu, flat)| {
                let vfs = materialize(&tree);
                let mut rules = FolderRules::default();
                rules.handling.flatten = flat;
                let cmp = compare_memory(&vfs, &rules);
                let mut defaults = ReportDefaults::new("xml");
                defaults.folder_display = DisplayOptions {
                    filter: filters[filter],
                    folders: FolderMode::CompareStructure,
                    ignore_unimportant: iu,
                    suppress_filters: false,
                };
                let doc = render(
                    ReportInput::Folder(&cmp),
                    ReportKind::Folder,
                    &ReportWords::default(),
                    &defaults,
                );
                xmlcheck::check(&doc).map_err(|e| {
                    TestCaseError::fail(format!("{e}\n{}", String::from_utf8_lossy(&doc)))
                })?;
                docs.lock().unwrap().push(doc);
                Ok(())
            },
        )
        .unwrap();
    docs.into_inner().unwrap()
}

/// A title with the characters a title can hold (a CR and an LF included).
fn super_line() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::sample::select(&['T', ' ', '\r', '\n', '&', '<', '\u{1}', '\u{ffff}'][..]),
        0..8,
    )
    .prop_map(|c| c.into_iter().collect())
}

#[test]
fn a7_7_every_golden_and_64_generated_documents_pass() {
    let goldens = goldens();
    assert!(goldens.len() > 35 + 25, "XML goldens: {}", goldens.len());
    for path in &goldens {
        let bytes = std::fs::read(path).unwrap();
        xmlcheck::check(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    let docs = generated();
    assert_eq!(docs.len(), 64);
    if !xmllint_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut files = goldens.clone();
    for (i, doc) in docs.iter().enumerate() {
        let path = tmp.path().join(format!("generated-{i:02}.xml"));
        std::fs::write(&path, doc).unwrap();
        files.push(path);
    }
    if let Err(e) = xmllint(&xsd(), &files) {
        let kept = tmp.keep();
        panic!(
            "xmllint rejects (documents kept in {}):\n{e}",
            kept.display()
        );
    }
}

/// The checks can fail: a removed required attribute, an unknown attribute, a broken schema.
#[test]
fn a7_7_the_checks_reject_what_they_must() {
    let golden = fixtures("text")
        .join("r07_unimportant_mix")
        .join("expected.xml");
    let text = std::fs::read_to_string(&golden).unwrap();
    let without = text.replacen(" state=\"changed\"", "", 1);
    let unknown = text.replacen("<row state=", "<row colour=\"red\" state=", 1);
    assert_ne!(without, text);
    assert_ne!(unknown, text);
    let e = xmlcheck::check(without.as_bytes()).unwrap_err();
    assert_eq!(e, "row without state");
    let e = xmlcheck::check(unknown.as_bytes()).unwrap_err();
    assert_eq!(e, "row: unknown attribute colour");
    let bad_value = text.replacen("state=\"changed\"", "state=\"moved\"", 1);
    assert!(xmlcheck::check(bad_value.as_bytes()).is_err());
    let misplaced = text.replacen("<rows>", "<rows>\n    <hidden rows=\"1\"><l/></hidden>", 1);
    assert!(xmlcheck::check(misplaced.as_bytes()).is_err());
    let raw_control = text.replacen("<l>", "<l>\u{1}", 1);
    assert!(
        xmlcheck::check(raw_control.as_bytes()).is_err(),
        "a raw control character is not well-formed"
    );
    if !xmllint_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| {
        let path = tmp.path().join(name);
        std::fs::write(&path, body).unwrap();
        path
    };
    let good = write("good.xml", &text);
    xmllint(&xsd(), std::slice::from_ref(&good)).expect("the golden validates");
    let e = xmllint(&xsd(), &[write("without.xml", &without)]).unwrap_err();
    assert!(e.contains("state") && e.contains("required"), "{e}");
    let e = xmllint(&xsd(), &[write("unknown.xml", &unknown)]).unwrap_err();
    assert!(e.contains("colour") && e.contains("not allowed"), "{e}");
    let e = xmllint(&xsd(), &[write("value.xml", &bad_value)]).unwrap_err();
    assert!(e.contains("moved"), "{e}");
    // A broken copy of the schema (a type it does not define) fails every document, the good one too.
    let schema = std::fs::read_to_string(xsd()).unwrap();
    let broken = schema.replacen("type=\"h:Bool\"", "type=\"h:Nothing\"", 1);
    assert_ne!(broken, schema);
    let broken = write("broken.xsd", &broken);
    let e = xmllint(&broken, &[good]).unwrap_err();
    assert!(!e.is_empty());
}
