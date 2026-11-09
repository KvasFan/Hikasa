//! M2 W10c acceptance A10.27 (TE-001, TC-064, D-289; contract §1.10.21): a Text Edit document through the Service —
//! open (format, encoding, stamp, editability), edit, save and Save As through `text_save_plan`/`text_save_execute`,
//! conversions (`hikasa_text::transform`) on the one document, the switches and the session's read-only flag, a
//! read-only file with a writable `--savetarget` (W10b note 59) on `MemoryVfs`; a `text-edit` session's keys round trip
//! (`[specs] left`, `left_format`, `left_encoding`, `read_only_left`); `text_compare_texts` choosing the formats by the
//! names, documents without a location (Save is Save As).

mod common;

use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::{ExitCode, JobKind, Side, Spec, WarningCode};
use hikasa_formats::DetectReason;
use hikasa_session::api::{
    NamedText, SaveAnswers, TextCompareRequest, TextEditRequest, TextEditResponse,
    apply_save_report, save_exit_code,
};
use hikasa_session::sessions::{ParentLink, SessionRef, SessionType};
use hikasa_session::{ApiError, Service};
use hikasa_text::save::{SavePlan, SaveReport, SaveRequest};
use hikasa_text::{
    EditBlock, EditRequest, EditableDocument, GroupHint, LineEnding, TextPos, Transform, transform,
};
use hikasa_vfs::{Attrs, MemMeta, MemoryVfs, UnixMeta};
use toml::Table;

// ---- helpers -------------------------------------------------------------------------------------------------------

fn request(path: &Path) -> TextEditRequest {
    TextEditRequest {
        spec: os(path),
        ..TextEditRequest::default()
    }
}

fn open(service: &Service, req: &TextEditRequest) -> TextEditResponse {
    service
        .text_edit_open(req, &job(service, JobKind::TextCompare))
        .unwrap()
}

fn type_at(doc: &mut EditableDocument, line: u32, text: &str) {
    let at = TextPos::new(line, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

/// Plans `doc`'s save with no side, answers every question yes, executes and applies the report.
fn save(service: &Service, doc: &mut EditableDocument, req: &SaveRequest) -> SaveReport {
    let control = job(service, JobKind::TextSave);
    let plan: SavePlan = service
        .text_save_plan(&[(None, &*doc, req)], &control)
        .unwrap();
    assert_eq!(plan.items[0].side, None, "a Text Edit document has no side");
    let plan = service
        .save_confirmation(
            &plan,
            SaveAnswers {
                yes: true,
                force: true,
                ..SaveAnswers::default()
            },
        )
        .finish(plan)
        .unwrap();
    let report = service.text_save_execute(plan, &[&*doc], &control).unwrap();
    apply_save_report(&report, &mut [doc]);
    report
}

fn spec(service: &Service, path: &Path) -> Spec {
    service.parse_spec(path.as_os_str()).unwrap()
}

/// A host path for `MemoryVfs`.
fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

const UNIX_RO: Attrs = Attrs::Unix {
    meta: UnixMeta {
        mode: 0o100_444,
        uid: 0,
        gid: 0,
    },
};

// ---- open, edit, save, Save As --------------------------------------------------------------------------------------------

#[test]
fn a10_27_open_edit_save_and_save_as() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let file = write(&w, "edit.c", b"int a;\nint b;\n");
    let mut resp = open(&service, &request(&file));
    // The format by the name (D-15), the stamp of the bytes read, an editable document saved in place.
    assert!(
        matches!(&resp.format.reason, DetectReason::Mask { id, .. } if id.as_str() == "c-cpp"),
        "{:?}",
        resp.format
    );
    assert_eq!(resp.stamp.unwrap().size, 14);
    assert!(resp.editability.editable());
    assert!(resp.editability.can_save_in_place);
    assert_eq!(resp.spec, spec(&service, &file));
    assert_eq!(resp.doc.location(), Some(&resp.spec));
    assert!(resp.warnings.is_empty(), "{:?}", resp.warnings);
    assert_eq!(resp.doc.line_count(), 2);
    // Edit and save in place.
    type_at(&mut resp.doc, 1, "// ");
    assert!(resp.doc.is_modified());
    let req = service.text_edit_save_request(&resp).unwrap();
    let report = save(&service, &mut resp.doc, &req);
    assert_eq!(save_exit_code(&report), ExitCode::Success);
    assert_eq!(std::fs::read(&file).unwrap(), b"int a;\n// int b;\n");
    assert!(!resp.doc.is_modified());
    // Save As moves the document to the new file; the old one stays.
    type_at(&mut resp.doc, 0, "X");
    let other = w.join("copy.c");
    let save_as = SaveRequest {
        target: Some(spec(&service, &other)),
        ..req.clone()
    };
    let report = save(&service, &mut resp.doc, &save_as);
    assert!(report.ok());
    assert_eq!(std::fs::read(&other).unwrap(), b"Xint a;\n// int b;\n");
    assert_eq!(std::fs::read(&file).unwrap(), b"int a;\n// int b;\n");
    assert_eq!(resp.doc.location(), Some(&spec(&service, &other)));
}

#[test]
fn a10_27_conversions_on_one_document() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let file = write(&w, "conv.txt", b"a  \r\n    b\r\nc\t\r\n");
    let mut resp = open(&service, &request(&file));
    let tab = resp.doc.options().tab_size;
    for t in [
        Transform::TrimTrailing,
        Transform::LeadingSpacesToTabs,
        Transform::LineEndings(LineEnding::Lf),
    ] {
        let outcome = transform(&resp.doc, t, None, tab);
        let edit = outcome.request.expect("each conversion changes the file");
        resp.doc.apply(&edit, GroupHint::None).unwrap();
    }
    let req = service.text_edit_save_request(&resp).unwrap();
    let report = save(&service, &mut resp.doc, &req);
    assert!(report.ok());
    let expected: &[u8] = if tab == 4 {
        b"a\n\tb\nc\n"
    } else {
        b"a\n    b\nc\n"
    };
    assert_eq!(std::fs::read(&file).unwrap(), expected, "tab size {tab}");
    // One undo step per conversion: three undo the line endings, the tabs and the trim.
    for _ in 0..3 {
        assert!(resp.doc.undo().is_some());
    }
    assert_eq!(
        resp.doc.document().rope().to_string(),
        "a  \r\n    b\r\nc\t\r\n"
    );
}

#[test]
fn a10_27_format_encoding_and_the_read_only_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let file = write(&w, "latin.txt", b"caf\xe9\n");
    // The encoding override and a forced format.
    let resp = open(
        &service,
        &TextEditRequest {
            encoding: Some("windows-1252".into()),
            format: Some("python".into()),
            ..request(&file)
        },
    );
    assert_eq!(resp.doc.line(hikasa_text::LineIdx(0)), "café");
    assert!(matches!(&resp.format.reason, DetectReason::Forced { id } if id.as_str() == "python"));
    // Without it the bytes decode some other way, never as an error.
    let resp = open(&service, &request(&file));
    assert_eq!(resp.doc.line_count(), 1);
    // `--ro` and the session's flag.
    let resp = open(
        &service,
        &TextEditRequest {
            read_only: true,
            session_read_only: true,
            parent_read_only: true,
            ..request(&file)
        },
    );
    assert_eq!(
        resp.editability.blocks,
        vec![
            EditBlock::ParentReadOnly,
            EditBlock::SessionReadOnly,
            EditBlock::CommandLine
        ]
    );
    assert_eq!(resp.doc.editability(), resp.editability);
    // A blocked Text Edit document refuses its save before planning (A10.6's rule, no side).
    let req = service.text_edit_save_request(&resp).unwrap();
    let err = service
        .text_save_plan(
            &[(None, &resp.doc, &req)],
            &job(&service, JobKind::TextSave),
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            ApiError::EditBlocked {
                side: None,
                block: EditBlock::ParentReadOnly
            }
        ),
        "{err:?}"
    );
    // A file that is missing cannot be loaded.
    let err = service
        .text_edit_open(
            &request(&w.join("missing.txt")),
            &job(&service, JobKind::TextCompare),
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::LoadFailed { .. }), "{err:?}");
    // Binary content opens blocked, with the warning.
    let bin = write(&w, "bin.dat", b"ab\0cd\0\x01\x02\n");
    let resp = open(&service, &request(&bin));
    assert_eq!(resp.editability.blocks, vec![EditBlock::Binary]);
    assert!(
        resp.warnings
            .iter()
            .any(|w| w.code == WarningCode::BinaryContent)
    );
}

#[test]
fn a10_27_the_save_target_decides_the_writability() {
    // W10b note 59 for a Text Edit document: a read-only file with a writable target is editable and saves there; a
    // read-only target makes it read-only.
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let vfs = MemoryVfs::new(hikasa_core::CaseSensitivity::Sensitive);
    let file = |path: &str, attrs: Option<Attrs>| {
        vfs.insert_file(
            &host(path),
            b"text\n".to_vec(),
            MemMeta {
                attrs,
                ..MemMeta::default()
            },
        )
        .unwrap();
    };
    file("/m/ro.txt", Some(UNIX_RO));
    file("/m/rw.txt", None);
    file("/m/target-ro.txt", Some(UNIX_RO));
    let control = job(&service, JobKind::TextCompare);
    let open_on = |spec: &str, target: Option<&str>| {
        service
            .text_edit_open_on(
                &vfs,
                &TextEditRequest {
                    spec: host(spec).into(),
                    save_target: target.map(host),
                    ..TextEditRequest::default()
                },
                &control,
            )
            .unwrap()
    };
    assert_eq!(
        open_on("/m/ro.txt", None).editability.blocks,
        vec![EditBlock::ReadOnlyLocation]
    );
    let resp = open_on("/m/ro.txt", Some("/m/rw.txt"));
    assert!(resp.editability.editable(), "{:?}", resp.editability);
    assert_eq!(
        resp.doc.save_target().map(Spec::canonical),
        Some(host("/m/rw.txt"))
    );
    assert_eq!(
        open_on("/m/rw.txt", Some("/m/target-ro.txt"))
            .editability
            .blocks,
        vec![EditBlock::ReadOnlyLocation]
    );
}

// ---- the text-edit session -------------------------------------------------------------------------------------------

#[test]
fn a10_27_a_text_edit_sessions_keys_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let file = write(&w, "notes.txt", b"n\n");
    let req = TextEditRequest {
        format: Some("python".into()),
        encoding: Some("utf-16le".into()),
        session_read_only: true,
        ..request(&file)
    };
    let parent = ParentLink {
        session: "_autosaved/1".into(),
        left: "notes.txt".into(),
        right: String::new(),
    };
    let (session, warnings) = service
        .text_edit_session(&req, Table::new(), Table::new(), Some(parent.clone()))
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(session.kind, SessionType::TextEdit);
    assert_eq!(session.parent, Some(parent));
    let reference = SessionRef::parse("edits/notes").unwrap();
    let written = service.session_save(&reference, &session, false).unwrap();
    // The keys in the file.
    let text = std::fs::read_to_string(&written.written[0]).unwrap();
    let doc: Table = toml::from_str(&text).unwrap();
    assert_eq!(doc["type"].as_str(), Some("text-edit"));
    let specs = doc["specs"].as_table().unwrap();
    assert_eq!(
        specs["left"].as_str(),
        Some(spec(&service, &file).canonical().as_str())
    );
    assert_eq!(specs["left_format"].as_str(), Some("python"));
    assert_eq!(specs["left_encoding"].as_str(), Some("utf-16le"));
    assert_eq!(specs["read_only_left"].as_bool(), Some(true));
    // Loaded again, the same request (the spec as stored: absolute and canonical).
    let loaded = service.session_load(&reference).unwrap().value;
    let back = service.text_edit_request_of(&loaded).unwrap();
    assert_eq!(
        back,
        TextEditRequest {
            spec: spec(&service, &file).canonical().into(),
            ..req.clone()
        }
    );
    assert_eq!(service.session_text_edit(&reference).unwrap(), back);
    // And it opens.
    let resp = open(
        &service,
        &TextEditRequest {
            encoding: None,
            ..back
        },
    );
    assert_eq!(resp.editability.blocks, vec![EditBlock::SessionReadOnly]);
    // Standard input is not stored; another session type is not a Text Edit.
    let (stdin, warnings) = service
        .text_edit_session(
            &TextEditRequest {
                spec: "-".into(),
                ..TextEditRequest::default()
            },
            Table::new(),
            Table::new(),
            None,
        )
        .unwrap();
    assert_eq!(stdin.specs.left, "");
    assert!(
        warnings.iter().any(|w| w.code == WarningCode::SpecNotSaved),
        "{warnings:?}"
    );
    assert!(service.text_edit_request_of(&stdin).is_err());
    let folder = hikasa_session::sessions::Session::builtin(SessionType::FolderCompare);
    assert!(service.text_edit_request_of(&folder).is_err());
}

// ---- text pairs ----------------------------------------------------------------------------------------------------

#[test]
fn a10_27_text_pairs_detect_the_format_by_name_and_save_as_only() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let named = |name: &str, text: &str| NamedText {
        name: name.into(),
        text: text.into(),
    };
    let control = job(&service, JobKind::TextCompare);
    let mut view = service
        .text_compare_texts(
            named("selection.c", "int a;\nint b;\n"),
            named("clipboard.c", "int a;\nint c;\n"),
            &TextCompareRequest::default(),
            &control,
        )
        .unwrap();
    assert!(
        matches!(&view.response.format.reason, DetectReason::Mask { id, .. } if id.as_str() == "c-cpp"),
        "{:?}",
        view.response.format
    );
    assert_eq!(view.live.current().stats().changed_rows, 1);
    for side in [Side::Left, Side::Right] {
        let doc = if side == Side::Left {
            &view.docs.left
        } else {
            &view.docs.right
        };
        assert!(doc.editability().editable());
        assert!(!doc.editability().can_save_in_place, "Save is Save As");
        assert_eq!(doc.location(), None);
    }
    assert_eq!(view.stamps.left, None);
    // A save without a target has nowhere to go: its item fails and asks for one.
    type_at(&mut view.docs.left, 0, "X");
    let in_place = SaveRequest::in_place(None, hikasa_text::save::UnmappablePolicy::Ask);
    let plan = service
        .text_save_plan(
            &[(Some(Side::Left), &view.docs.left, &in_place)],
            &job(&service, JobKind::TextSave),
        )
        .unwrap();
    let report = service
        .text_save_execute(plan, &[&view.docs.left], &job(&service, JobKind::TextSave))
        .unwrap();
    assert!(!report.ok());
    // Save As writes the edited text.
    let target = tmp.path().join("work").join("saved.c");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let save_as = SaveRequest {
        target: Some(spec(&service, &target)),
        ..in_place
    };
    let plan = service
        .text_save_plan(
            &[(Some(Side::Left), &view.docs.left, &save_as)],
            &job(&service, JobKind::TextSave),
        )
        .unwrap();
    let report = service
        .text_save_execute(plan, &[&view.docs.left], &job(&service, JobKind::TextSave))
        .unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(std::fs::read(&target).unwrap(), b"Xint a;\nint b;\n");

    // Names of two formats: the left one's is used for both, with `format_mismatch`; no name: Everything Else.
    let view = service
        .text_compare_texts(
            named("a.c", "x\n"),
            named("b.py", "x\n"),
            &TextCompareRequest::default(),
            &control,
        )
        .unwrap();
    assert!(
        view.response
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::FormatMismatch),
        "{:?}",
        view.response.warnings
    );
    let view = service
        .text_compare_texts(
            named("", "x\n"),
            named("", "x\n"),
            &TextCompareRequest::default(),
            &control,
        )
        .unwrap();
    assert_eq!(view.response.format.reason, DetectReason::Fallback);
    // Per-side formats and anchors of the request apply as in any comparison.
    let req = TextCompareRequest {
        formats: hikasa_core::Pair::new(Some("python".into()), None),
        ..TextCompareRequest::default()
    };
    let view = service
        .text_compare_texts(named("a.c", "x\n"), named("b.c", "y\n"), &req, &control)
        .unwrap();
    assert!(view.response.right_format.is_some());
}
