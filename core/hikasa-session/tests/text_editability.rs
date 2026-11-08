//! A10.6 (D-265, contract §1.10.7): the editability of each pane of a live comparison, row by row of the table;
//! `apply` refuses with the first block; `ApiError::EditBlocked` (exit 100) through `ensure_editable`. Also the
//! editing options a live view gets (A10.2's `default_line_ending` part, D-290) and the request's anchors
//! (§1.10.8).

mod common;

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use common::{job, os, service, service_with, write};
use hikasa_core::{ErrorKind, ExitCode, JobKind, Side};
use hikasa_folder::ops::ReadOnlySides;
use hikasa_session::api::{
    ApiOp, EditFacts, LiveOptions, LiveTextView, TextCompareRequest, edit_options, ensure_editable,
};
use hikasa_session::host::Platform;
use hikasa_session::ipc::ReadOnly;
use hikasa_session::sessions::{SessionRef, SessionType};
use hikasa_session::settings::options::Options;
use hikasa_session::{ApiError, Service};
use hikasa_text::{
    Anchor, AnchorSide, EditBlock, EditError, EditRequest, Editability, GroupHint, LineEnding,
    TextPos,
};
use hikasa_vfs::{Attrs, Capabilities, DosAttrs, MemMeta, MemoryVfs, UnixMeta};

/// A request for two paths.
fn req(left: &Path, right: &Path) -> TextCompareRequest {
    TextCompareRequest {
        left: os(left),
        right: os(right),
        ..TextCompareRequest::default()
    }
}

/// Opens a live comparison.
fn open(service: &Service, req: &TextCompareRequest, opts: LiveOptions) -> LiveTextView {
    service
        .text_compare_live(req, opts, &job(service, JobKind::TextCompare), None)
        .unwrap()
}

/// The blocks of each pane.
fn blocks(view: &LiveTextView) -> (Vec<EditBlock>, Vec<EditBlock>) {
    (
        view.docs.left.editability().blocks,
        view.docs.right.editability().blocks,
    )
}

/// One character typed at the start of the document.
fn type_x() -> EditRequest {
    EditRequest::Replace {
        range: TextPos::new(0, 0)..TextPos::new(0, 0),
        text: "x".into(),
        keep_eols: false,
    }
}

/// A host path for `MemoryVfs`: `/a/b` on Unix, `C:\a\b` on Windows.
fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// A file of `MemoryVfs` with the given attributes.
fn mem_file(vfs: &MemoryVfs, path: &str, text: &str, attrs: Option<Attrs>) {
    vfs.insert_file(
        &host(path),
        text.as_bytes().to_vec(),
        MemMeta {
            attrs,
            ..MemMeta::default()
        },
    )
    .unwrap();
}

/// Opens `left` and `right` of `vfs` live.
fn open_on(
    service: &Service,
    vfs: &MemoryVfs,
    left: &str,
    right: &str,
    opts: LiveOptions,
) -> LiveTextView {
    let req = TextCompareRequest {
        left: host(left).into(),
        right: host(right).into(),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live_on(vfs, &req, opts, &job(service, JobKind::TextCompare), None)
        .unwrap()
}

const UNIX_RO: Attrs = Attrs::Unix {
    meta: UnixMeta {
        mode: 0o100_444,
        uid: 0,
        gid: 0,
    },
};

#[test]
fn memory_vfs_read_only_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let vfs = MemoryVfs::new(hikasa_core::CaseSensitivity::Sensitive);
    mem_file(&vfs, "/m/rw.txt", "a\n", None);
    mem_file(&vfs, "/m/ro.txt", "b\n", Some(UNIX_RO));
    mem_file(
        &vfs,
        "/m/dos.txt",
        "c\n",
        Some(Attrs::Dos {
            attrs: DosAttrs::READONLY,
        }),
    );
    let view = open_on(
        &service,
        &vfs,
        "/m/rw.txt",
        "/m/ro.txt",
        LiveOptions::default(),
    );
    assert_eq!(blocks(&view), (vec![], vec![EditBlock::ReadOnlyLocation]));
    assert!(view.docs.right.editability().can_save_in_place);
    assert_eq!(view.docs.right.editability().status_text(), "Read-only");
    let view = open_on(
        &service,
        &vfs,
        "/m/dos.txt",
        "/m/rw.txt",
        LiveOptions::default(),
    );
    assert_eq!(blocks(&view), (vec![EditBlock::ReadOnlyLocation], vec![]));
    // A tree without the write capability: every file.
    vfs.set_capabilities(Capabilities::empty());
    let view = open_on(
        &service,
        &vfs,
        "/m/rw.txt",
        "/m/rw.txt",
        LiveOptions::default(),
    );
    assert_eq!(
        blocks(&view),
        (
            vec![EditBlock::ReadOnlyLocation],
            vec![EditBlock::ReadOnlyLocation]
        )
    );
}

/// True when this process can open `path` for writing although its permissions say read-only (root, a file
/// system that ignores the bits).
fn writes_anyway(path: &Path) -> bool {
    std::fs::OpenOptions::new().append(true).open(path).is_ok()
}

#[test]
fn disk_read_only_file() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let ro = write(&w, "ro.txt", b"one\n");
    let rw = write(&w, "rw.txt", b"two\n");
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro, perms).unwrap();
    let view = open(&service, &req(&rw, &ro), LiveOptions::default());
    if writes_anyway(&ro) {
        println!(
            "skipped the ReadOnlyLocation assertion: this process can write a read-only file (root?), so access(W_OK) \
             says writable"
        );
    } else {
        assert_eq!(blocks(&view), (vec![], vec![EditBlock::ReadOnlyLocation]));
    }
    assert!(view.docs.left.editability().editable());
    // Restore the bit so the temporary folder can be removed everywhere.
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&ro, perms).unwrap();
}

/// A zip of stored entries.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// CRC-32 (IEEE) of `data`, for the gzip trailer.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A gzip stream of one stored block.
fn gz_of(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1];
    let len = u16::try_from(data.len()).unwrap();
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

#[test]
fn archive_members() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let zip = write(&w, "a.zip", &zip_of(&[("x.txt", b"in zip\n")]));
    let gz = write(&w, "n.txt.gz", &gz_of(b"in gz\n"));
    let member = |archive: &Path, name: &str| archive.join(name);
    let view = open(
        &service,
        &req(&member(&zip, "x.txt"), &member(&gz, "n.txt")),
        LiveOptions::default(),
    );
    assert!(
        matches!(view.specs.left, hikasa_core::Spec::ArchiveMember { .. }),
        "{:?}",
        view.specs.left
    );
    // zip writes (D-63); a single-stream gzip cannot.
    assert_eq!(blocks(&view), (vec![], vec![EditBlock::ArchiveReadOnly]));
    assert!(view.docs.left.editability().can_save_in_place);
    assert_eq!(view.docs.right.editability().status_text(), "Read-only");
}

/// A user format converted by the fake runner (upper case), with a `save` command line or without.
fn external_format(save: bool) -> String {
    let mut text = common::EXTERNAL_FORMAT.to_owned();
    if save {
        text.push_str("save = \"conv %s %t\"\n");
    }
    text
}

#[test]
fn converted_formats() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config");
    write(
        &config,
        "formats/upper.toml",
        external_format(false).as_bytes(),
    );
    write(
        &config,
        "formats/saved.toml",
        external_format(true)
            .replace("id = \"upper\"", "id = \"saved\"")
            .replace("name = \"Upper\"", "name = \"Saved\"")
            .replace("*.up", "*.sv")
            .as_bytes(),
    );
    let service = service_with(tmp.path(), None, Arc::new(common::UpperRunner::default()));
    assert!(
        service.startup_warnings().is_empty(),
        "{:?}",
        service.startup_warnings()
    );
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"b\na\n");
    let b = write(&w, "b.txt", b"a\nb\n");
    // Sorted.
    let mut sorted = req(&a, &b);
    sorted.format = Some("sorted".into());
    let view = open(&service, &sorted, LiveOptions::default());
    let converted = vec![EditBlock::Converted];
    assert_eq!(blocks(&view), (converted.clone(), converted.clone()));
    // External without `save`.
    let up = (write(&w, "a.up", b"x\n"), write(&w, "b.up", b"y\n"));
    let view = open(&service, &req(&up.0, &up.1), LiveOptions::default());
    assert_eq!(view.response.format.name, "Upper");
    assert_eq!(blocks(&view), (converted.clone(), converted));
    // External with `save`: editable (its save runs that program).
    let sv = (write(&w, "a.sv", b"x\n"), write(&w, "b.sv", b"y\n"));
    let view = open(&service, &req(&sv.0, &sv.1), LiveOptions::default());
    assert_eq!(view.response.format.name, "Saved");
    assert_eq!(blocks(&view), (vec![], vec![]));
}

#[test]
fn switches_parent_and_session() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let pair = req(&write(&w, "a.txt", b"a\n"), &write(&w, "b.txt", b"b\n"));
    // `--ro2`.
    let ro2 = ReadOnly {
        right: true,
        ..ReadOnly::default()
    };
    let view = open(
        &service,
        &pair,
        LiveOptions {
            read_only: ro2.sides(),
            ..LiveOptions::default()
        },
    );
    assert_eq!(blocks(&view), (vec![], vec![EditBlock::CommandLine]));
    assert_eq!(
        view.docs.right.editability().status_text(),
        "Editing disabled"
    );
    // `--ro`: both.
    let all = ReadOnly {
        all: true,
        ..ReadOnly::default()
    };
    assert_eq!(
        all.sides(),
        ReadOnlySides {
            left: true,
            right: true
        }
    );
    // The parent folder comparison's left side.
    let view = open(
        &service,
        &pair,
        LiveOptions {
            parent_read_only: ReadOnlySides {
                left: true,
                right: false,
            },
            ..LiveOptions::default()
        },
    );
    assert_eq!(blocks(&view), (vec![EditBlock::ParentReadOnly], vec![]));
    // The session's `[specs] read_only_left`.
    let reference = SessionRef::parse("ro").unwrap();
    let mut session = service.session_defaults(SessionType::TextCompare).unwrap();
    session.specs.left = w.join("a.txt").display().to_string();
    session.specs.right = w.join("b.txt").display().to_string();
    session.specs.read_only_left = true;
    service.session_save(&reference, &session, false).unwrap();
    let view = open(
        &service,
        &pair,
        LiveOptions {
            session_read_only: service.session_read_only(&reference).unwrap(),
            ..LiveOptions::default()
        },
    );
    assert_eq!(blocks(&view), (vec![EditBlock::SessionReadOnly], vec![]));
    // The options are kept for the saves (W10b).
    let view = open(
        &service,
        &pair,
        LiveOptions {
            save_target: Some("/x/out.txt".into()),
            no_backups: true,
            ..LiveOptions::default()
        },
    );
    assert_eq!(view.options.save_target.as_deref(), Some("/x/out.txt"));
    assert!(view.options.no_backups);
}

#[test]
fn cancelled_and_failed_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let pair = req(&write(&w, "a.txt", b"a\n"), &write(&w, "b.txt", b"b\n"));
    // A cancelled comparison opens no view; the view that keeps its panes marks them with the same rule.
    let control = job(&service, JobKind::TextCompare);
    control.cancel.cancel();
    let err = service
        .text_compare_live(&pair, LiveOptions::default(), &control, None)
        .unwrap_err();
    assert!(matches!(err, ApiError::Cancelled), "{err:?}");
    let cancelled = EditFacts {
        cancelled: true,
        has_location: true,
        ..EditFacts::default()
    };
    assert_eq!(cancelled.editability().blocks, vec![EditBlock::Cancelled]);
    assert_eq!(cancelled.editability().status_text(), "Editing disabled");
    // A side that cannot be loaded fails the call (exit 100) …
    let missing = req(&w.join("a.txt"), &w.join("missing.txt"));
    let err = service
        .text_compare_live(
            &missing,
            LiveOptions::default(),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            ApiError::LoadFailed {
                side: Side::Right,
                ..
            }
        ),
        "{err:?}"
    );
    // … and the view marks its pane `LoadFailed`, first of every cause.
    let failed = EditFacts {
        load_failed: true,
        cancelled: true,
        command_line: true,
        ..EditFacts::default()
    };
    assert_eq!(
        failed.editability().blocks,
        vec![
            EditBlock::LoadFailed,
            EditBlock::Cancelled,
            EditBlock::CommandLine
        ]
    );
    assert_eq!(failed.editability().status_text(), "");
}

#[test]
fn edit_facts_table() {
    let base = EditFacts {
        has_location: true,
        ..EditFacts::default()
    };
    // `Vfs::writable` = None (cannot tell) is writable: the save finds out.
    for writable in [None, Some(true)] {
        let facts = EditFacts { writable, ..base };
        assert_eq!(facts.editability(), Editability::EDITABLE, "{writable:?}");
        let member = EditFacts {
            archive_member: true,
            ..facts
        };
        assert_eq!(member.editability(), Editability::EDITABLE, "{writable:?}");
    }
    let rows = [
        (
            EditFacts {
                writable: Some(false),
                ..base
            },
            EditBlock::ReadOnlyLocation,
        ),
        (
            EditFacts {
                writable: Some(false),
                archive_member: true,
                ..base
            },
            EditBlock::ArchiveReadOnly,
        ),
        (
            EditFacts {
                converted: true,
                ..base
            },
            EditBlock::Converted,
        ),
        (
            EditFacts {
                truncated: true,
                ..base
            },
            EditBlock::Truncated,
        ),
        (
            EditFacts {
                parent_read_only: true,
                ..base
            },
            EditBlock::ParentReadOnly,
        ),
        (
            EditFacts {
                session_read_only: true,
                ..base
            },
            EditBlock::SessionReadOnly,
        ),
        (
            EditFacts {
                command_line: true,
                ..base
            },
            EditBlock::CommandLine,
        ),
        (
            EditFacts {
                binary: true,
                ..base
            },
            EditBlock::Binary,
        ),
    ];
    for (facts, block) in rows {
        let e = facts.editability();
        assert_eq!(e.blocks, vec![block], "{facts:?}");
        assert!(e.can_save_in_place);
    }
    // Without a location: editable, Save becomes Save As.
    let stdin = EditFacts::default().editability();
    assert!(stdin.editable() && !stdin.can_save_in_place);
}

#[test]
fn binary_content_and_edit_anyway() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let bin = write(&w, "a.bin", b"ab\0cd\0\x01\x02\n");
    let text = write(&w, "b.txt", b"text\n");
    let mut view = open(&service, &req(&bin, &text), LiveOptions::default());
    assert_eq!(blocks(&view), (vec![EditBlock::Binary], vec![]));
    assert_eq!(
        view.docs
            .left
            .apply(&type_x(), GroupHint::None)
            .unwrap_err(),
        EditError::Blocked(EditBlock::Binary)
    );
    view.docs.left.allow_binary_edits();
    assert!(view.docs.left.editability().editable());
    view.docs.left.apply(&type_x(), GroupHint::None).unwrap();
    assert!(view.docs.left.is_modified());
    // Lifting it is the view's choice only: a new open blocks again.
    let view = open(&service, &req(&bin, &text), LiveOptions::default());
    assert_eq!(blocks(&view).0, vec![EditBlock::Binary]);
}

#[test]
fn stdin_has_no_location() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service_with(
        tmp.path(),
        Some(common::blob(b"from stdin\n", Some("in.txt"))),
        Arc::new(common::UpperRunner::default()),
    );
    let file = write(&tmp.path().join("work"), "b.txt", b"file\n");
    let mut view = open(
        &service,
        &TextCompareRequest {
            left: "-".into(),
            right: os(&file),
            ..TextCompareRequest::default()
        },
        LiveOptions::default(),
    );
    let stdin = view.docs.left.editability();
    assert!(stdin.editable() && !stdin.can_save_in_place, "{stdin:?}");
    assert!(view.docs.right.editability().can_save_in_place);
    view.docs.left.apply(&type_x(), GroupHint::None).unwrap();
}

#[test]
fn apply_refuses_with_the_first_block() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let vfs = MemoryVfs::new(hikasa_core::CaseSensitivity::Sensitive);
    mem_file(&vfs, "/m/ro.txt", "b\0inary\n", Some(UNIX_RO));
    mem_file(&vfs, "/m/rw.txt", "a\n", None);
    let both = ReadOnlySides {
        left: true,
        right: false,
    };
    let mut view = open_on(
        &service,
        &vfs,
        "/m/ro.txt",
        "/m/rw.txt",
        LiveOptions {
            read_only: both,
            parent_read_only: both,
            ..LiveOptions::default()
        },
    );
    let left = view.docs.left.editability();
    assert_eq!(
        left.blocks,
        vec![
            EditBlock::ReadOnlyLocation,
            EditBlock::ParentReadOnly,
            EditBlock::CommandLine,
            EditBlock::Binary
        ]
    );
    let before = view.docs.left.generation();
    assert_eq!(
        view.docs
            .left
            .apply(&type_x(), GroupHint::None)
            .unwrap_err(),
        EditError::Blocked(EditBlock::ReadOnlyLocation)
    );
    assert_eq!(view.docs.left.generation(), before, "nothing changed");
    // Lifting Binary leaves the others.
    view.docs.left.allow_binary_edits();
    assert_eq!(
        view.docs
            .left
            .apply(&type_x(), GroupHint::None)
            .unwrap_err(),
        EditError::Blocked(EditBlock::ReadOnlyLocation)
    );
    // The right pane edits.
    view.docs.right.apply(&type_x(), GroupHint::None).unwrap();
}

#[test]
fn edit_blocked_is_permission_denied_exit_100() {
    assert!(ensure_editable(Some(Side::Left), &Editability::EDITABLE).is_ok());
    let blocked = Editability::with_blocks([EditBlock::CommandLine, EditBlock::Converted], true);
    let err = ensure_editable(Some(Side::Right), &blocked).unwrap_err();
    assert!(
        matches!(
            err,
            ApiError::EditBlocked {
                side: Some(Side::Right),
                block: EditBlock::Converted
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    for op in [ApiOp::TextCompare, ApiOp::Other, ApiOp::FolderOp] {
        assert_eq!(err.exit_code(op), ExitCode::UnknownError, "{op:?}");
    }
    assert_eq!(ExitCode::UnknownError.code(), 100);
    assert_eq!(
        err.to_string(),
        "the right document cannot be edited: its format converts it without a way to save it back"
    );
    let report = err.report();
    assert_eq!(report.kind, ErrorKind::PermissionDenied);
    assert_eq!(report.spec, None);
    let text_edit = ensure_editable(
        None,
        &Editability::with_blocks([EditBlock::ReadOnlyLocation], true),
    )
    .unwrap_err();
    assert_eq!(
        text_edit.to_string(),
        "the document cannot be edited: its location is read-only"
    );
}

#[test]
fn edit_options_come_from_the_format_and_the_settings() {
    let tmp = tempfile::tempdir().unwrap();
    // A10.2 (D-290): `platform` resolved by fixed host facts, both ways.
    let format = hikasa_formats::CompiledFormat::everything_else();
    let mut options = Options::default();
    let mut facts = common::facts(tmp.path());
    for (platform, eol) in [
        (Platform::Windows, LineEnding::CrLf),
        (Platform::Linux, LineEnding::Lf),
        (Platform::MacOs, LineEnding::Lf),
    ] {
        facts.platform = platform;
        assert_eq!(
            edit_options(&format, &options, facts.platform).default_eol,
            eol
        );
    }
    options.text_editing.default_line_ending =
        hikasa_session::settings::options::DefaultLineEnding::Cr;
    options.text_editing.auto_indent = false;
    let opts = edit_options(&format, &options, Platform::Windows);
    assert_eq!(opts.default_eol, LineEnding::Cr);
    assert!(!opts.auto_indent && opts.backspace_unindents);
    // Through the service: the host's platform, then `options.toml`.
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let empty = write(&w, "empty.c", b"");
    let crlf = write(&w, "crlf.c", b"int x;\r\n");
    let view = open(&service, &req(&empty, &crlf), LiveOptions::default());
    let native = if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    assert_eq!(view.docs.left.insert_eol(), native, "no terminated line");
    assert_eq!(view.docs.right.insert_eol(), LineEnding::CrLf, "dominant");
    let registry = service.registry();
    let c = &registry.find("c-cpp").unwrap().format;
    assert_eq!(view.response.format.name, c.name);
    assert_eq!(view.docs.left.options().tab_size, c.text.tab_size);
    assert_eq!(view.docs.left.options().word_chars, c.text.word_chars);
    assert!(view.docs.left.options().auto_indent);
    write(
        &tmp.path().join("config"),
        "options.toml",
        b"[text_editing]\ndefault_line_ending = \"cr\"\nbackspace_unindents = false\n",
    );
    let view = open(&service, &req(&empty, &crlf), LiveOptions::default());
    assert_eq!(view.docs.left.insert_eol(), LineEnding::Cr);
    assert!(!view.docs.left.options().backspace_unindents);
}

#[test]
fn the_request_anchors_reach_the_comparison() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"one\ntwo\nthree\nfour\n");
    let b = write(&w, "b.txt", b"zero\none\ntwo\nTHREE\nfour\n");
    let mut request = req(&a, &b);
    request.anchors = vec![
        Anchor::Pair {
            left: 2..3,
            right: 3..4,
        },
        Anchor::Isolate {
            side: AnchorSide::Left,
            lines: 0..1,
        },
    ];
    let response = service
        .text_compare(&request, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(response.comparison.anchors(), request.anchors.as_slice());
    let view = open(&service, &request, LiveOptions::default());
    assert_eq!(view.live.anchors(), request.anchors.as_slice());
    assert!(Arc::ptr_eq(&view.response.comparison, view.live.current()));
    // A crossing pair is dropped with a warning.
    request.anchors.push(Anchor::Pair {
        left: 3..4,
        right: 0..1,
    });
    let response = service
        .text_compare(&request, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(response.comparison.anchors().len(), 2);
    assert!(
        response
            .warnings
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::AnchorDropped),
        "{:?}",
        response.warnings
    );
    // A line beyond a document is invalid input, before anything is compared.
    for anchor in [
        Anchor::Pair {
            left: 0..5,
            right: 0..1,
        },
        Anchor::Pair {
            left: 0..1,
            right: 5..6,
        },
        Anchor::Isolate {
            side: AnchorSide::Right,
            lines: 4..6,
        },
    ] {
        request.anchors = vec![anchor.clone()];
        let err = service
            .text_compare(&request, &job(&service, JobKind::TextCompare), None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{anchor:?}");
        assert!(err.to_string().contains("beyond"), "{err}");
        let err = service
            .text_compare_live(
                &request,
                LiveOptions::default(),
                &job(&service, JobKind::TextCompare),
                None,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{anchor:?}");
    }
}
