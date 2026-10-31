//! The session's part of archives (§1.5.12, A5.13): specs, `infer_view_kind`, sessions, packages, settings, exit.

mod common;

use std::io::Write;

use common::{job, os, service, write};
use hikasa_core::{JobKind, Spec};
use hikasa_session::api::{TextCompareRequest, ViewRequest, infer_view_kind};
use hikasa_session::ipc::ViewKind;
use hikasa_session::package::ImportChoices;
use hikasa_session::sessions::SessionRef;
use hikasa_text::ViewOptions;
use hikasa_vfs::{FollowLinks, ReadSeek, SpecUse, Vfs};
use toml::Table;

fn zip_bytes(files: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(method);
    for (name, data) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip_bytes(files, zip::CompressionMethod::Stored)
}

fn read_all(mut reader: Box<dyn ReadSeek>) -> Vec<u8> {
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    out
}

#[test]
fn parse_spec_resolves_and_keeps_files() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let file = write(&work, "a.zip", &stored(&[("inner/x", b"member")]));
    let local = svc.parse_spec(file.as_os_str()).unwrap();
    assert!(matches!(local, Spec::Local(_)), "{local:?}");
    let root = svc
        .parse_spec_as(file.as_os_str(), SpecUse::Folder)
        .unwrap();
    assert_eq!(root, Spec::archive_member(local.clone(), ""));
    let mut slash = file.clone().into_os_string();
    slash.push(std::path::MAIN_SEPARATOR_STR);
    assert_eq!(svc.parse_spec(&slash).unwrap(), root);
    let member = svc
        .parse_spec(file.join("inner").join("x").as_os_str())
        .unwrap();
    assert_eq!(member, Spec::archive_member(local, "inner/x"));
    assert_eq!(read_all(svc.vfs().open_read(&member).unwrap()), b"member");
    assert_eq!(svc.vfs().stat(&member, FollowLinks::No).unwrap().size, 6);
}

/// §1.5.12: two archives by name, or an archive and a folder, are a Folder Compare; `--fv` wins.
#[test]
fn infer_view_kind_of_archives() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let a = write(&work, "a.zip", &stored(&[("x", b"1")]));
    let b = write(&work, "b.tar", &[0u8; 1024]);
    let text = write(&work, "t.txt", b"text\n");
    let dir = work.join("d");
    std::fs::create_dir_all(&dir).unwrap();
    let spec = |p: &std::path::Path| svc.parse_spec(p.as_os_str()).unwrap();
    let root = svc.parse_spec_as(a.as_os_str(), SpecUse::Folder).unwrap();
    let member = spec(&a.join("x"));
    for (left, right, want) in [
        (spec(&a), spec(&b), Some(ViewKind::FolderCompare)),
        (spec(&a), spec(&dir), Some(ViewKind::FolderCompare)),
        (spec(&dir), spec(&a), Some(ViewKind::FolderCompare)),
        (root.clone(), spec(&dir), Some(ViewKind::FolderCompare)),
        (root, spec(&b), Some(ViewKind::FolderCompare)),
        (spec(&a), spec(&text), Some(ViewKind::TextCompare)),
        (member, spec(&text), Some(ViewKind::TextCompare)),
        (spec(&text), spec(&dir), None),
    ] {
        let got = infer_view_kind(&svc, &[left.clone(), right.clone()], None);
        assert_eq!(got.ok(), want, "{left} {right}");
    }
    let forced = infer_view_kind(&svc, &[spec(&a), spec(&b)], Some(ViewKind::TextCompare));
    assert_eq!(forced.unwrap(), ViewKind::TextCompare);
}

/// D-80: specs are stored as canonical text, so a root and a member round-trip.
#[test]
fn sessions_round_trip_archive_specs() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let a = write(&work, "a.zip", &stored(&[("x", b"left\n")]));
    let b = write(&work, "b.txt", b"right\n");
    let dir = work.join("d");
    std::fs::create_dir_all(&dir).unwrap();

    let mut slash = a.clone().into_os_string();
    slash.push(std::path::MAIN_SEPARATOR_STR);
    let root = svc.parse_spec(&slash).unwrap();
    let (stored_root, warning) = svc.stored_spec(&slash.to_string_lossy()).unwrap();
    assert_eq!(warning, None);
    assert_eq!(stored_root, root.canonical());
    assert!(stored_root.ends_with('/'), "{stored_root}");
    let back = svc.parse_spec(std::ffi::OsStr::new(&stored_root)).unwrap();
    assert_eq!(back, root);

    // Text Compare on `a.zip/x`.
    let member = a.join("x");
    let request = TextCompareRequest {
        left: os(&member),
        right: os(&b),
        ..TextCompareRequest::default()
    };
    let view = ViewRequest::Text {
        request,
        view: ViewOptions::default(),
    };
    let (session, warnings) = svc
        .session_for_view(&view, Table::new(), Table::new(), None)
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let name = SessionRef::parse("Archive").unwrap();
    svc.session_save(&name, &session, false).unwrap();
    let loaded = svc.session_load(&name).unwrap();
    let parsed = svc
        .parse_spec(std::ffi::OsStr::new(&loaded.value.specs.left))
        .unwrap();
    let want = Spec::archive_member(svc.parse_spec(a.as_os_str()).unwrap(), "x");
    assert_eq!(parsed, want);
    let compared = svc
        .text_compare(
            &TextCompareRequest {
                left: std::ffi::OsString::from(&loaded.value.specs.left),
                right: os(&b),
                ..TextCompareRequest::default()
            },
            &job(&svc, JobKind::TextCompare),
            None,
        )
        .unwrap();
    let stats = compared.comparison.stats();
    assert_eq!(
        (stats.rows > 0, stats.same_rows),
        (true, 0),
        "the member was read"
    );
}

/// D-86 is unchanged: the workspace `zip` has more codecs now, and the import still takes stored and deflate only.
#[test]
fn a_package_with_a_bzip2_entry_is_still_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let path = tmp.path().join("p.hikasa-pkg");
    let manifest = "schema_version = 1\nformat = \"hikasa-pkg\"\nhikasa_version = \"0.2.0\"\n\
        created = \"2026-10-28T12:00:00+01:00\"\n";
    let bytes = zip_bytes(
        &[("manifest.toml", manifest.as_bytes())],
        zip::CompressionMethod::Bzip2,
    );
    std::fs::write(&path, bytes).unwrap();
    let error = svc
        .settings_import(&path, &ImportChoices::default(), false)
        .unwrap_err();
    assert_eq!(error.kind().as_str(), "invalid_data", "{error}");
    assert!(error.to_string().contains("stored and deflate"), "{error}");
}

#[test]
fn reload_settings_applies_new_masks() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let a = write(&work, "a.mine", &stored(&[("x", b"1")]));
    let member = a.join("x");
    assert!(matches!(
        svc.parse_spec(member.as_os_str()).unwrap(),
        Spec::Local(_)
    ));
    write(
        &svc.config().dir,
        "archives.toml",
        b"schema_version = 1\n[types]\nzip = \"*.mine\"\n",
    );
    assert!(svc.reload_settings().is_empty());
    let spec = svc.parse_spec(member.as_os_str()).unwrap();
    assert!(matches!(spec, Spec::ArchiveMember { .. }), "{spec:?}");
    assert_eq!(read_all(svc.vfs().open_read(&spec).unwrap()), b"1");
}

/// The location (`resolve_config_dir` decides `cache_dir`, tested per mode in `config_paths.rs`) and the exit.
#[test]
fn shutdown_removes_the_cache_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let big = vec![7u8; (1 << 20) + 1];
    let a = write(&work, "a.zip", &stored(&[("m", &big)]));
    let spec = svc.parse_spec(a.join("m").as_os_str()).unwrap();
    let reader = svc.vfs().open_read(&spec).unwrap();
    let cache = svc.config().cache_dir.join("archives");
    assert_eq!(
        std::fs::read_dir(&cache).unwrap().count(),
        2,
        "s<n>, its lock"
    );
    drop(reader);
    svc.shutdown();
    assert!(!cache.exists(), "nothing is left, not even an empty folder");
}

#[test]
fn input_stamps_of_members_read_no_content() {
    use hikasa_session::watch::InputState;
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let a = write(&work, "a.zip", &stored(&[("x", b"1234")]));
    let member = a.join("x");
    let state = svc.input_stamp(&member.to_string_lossy());
    assert!(
        matches!(&state, InputState::Present(stamp) if stamp.len == 4),
        "{state:?}"
    );
    let missing = svc.input_stamp(&a.join("nope").to_string_lossy());
    assert!(matches!(missing, InputState::Missing), "{missing:?}");
    let _ = svc.vfs().capabilities(&Spec::archive_member(
        svc.parse_spec(a.as_os_str()).unwrap(),
        "",
    ));
}

/// A cancelled job stops at the index on every path that reaches one (G4): `quick`, `text compare`, a folder
/// comparison before its scan, `vfs stat`, `vfs cat`.
#[test]
fn a_cancelled_job_stops_at_the_index() {
    use hikasa_session::api::{ApiError, FolderCompareRequest, QuickMethod, QuickRequest};

    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let work = tmp.path().join("work");
    let file = write(&work, "a.zip", &stored(&[("x", b"one")]));
    let member = file.join("x");
    let plain = write(&work, "plain.txt", b"one");
    let cancelled = || {
        let job = job(&svc, JobKind::VfsRead);
        job.cancel.cancel();
        job
    };
    let quick = QuickRequest {
        left: os(&member),
        right: os(&plain),
        method: QuickMethod::Binary,
        ..QuickRequest::default()
    };
    assert!(matches!(
        svc.quick(&quick, &cancelled()),
        Err(ApiError::Cancelled)
    ));
    let stat = svc.vfs_stat_in(member.as_os_str(), FollowLinks::Yes, &cancelled());
    assert!(matches!(stat, Err(ApiError::Cancelled)), "{stat:?}");
    let cat = svc.vfs_cat(member.as_os_str(), &mut Vec::new(), &cancelled());
    assert!(matches!(cat, Err(ApiError::Cancelled)), "{cat:?}");
    let text = TextCompareRequest {
        left: os(&member),
        right: os(&plain),
        ..TextCompareRequest::default()
    };
    let text = svc.text_compare(&text, &cancelled(), None);
    assert!(matches!(text, Err(ApiError::Cancelled)), "{text:?}");
    // Nested: the base resolves through the outer archive, inside the job.
    let nested = write(
        &work,
        "n.zip",
        &stored(&[("inner.zip", &stored(&[("y", b"z")]))]),
    );
    let folder = FolderCompareRequest {
        left: os(&nested.join("inner.zip")),
        right: os(&work),
        ..FolderCompareRequest::default()
    };
    let prepared = svc.folder_prepare_in(&folder, &cancelled());
    assert!(matches!(prepared, Err(ApiError::Cancelled)), "{prepared:?}");
    assert_eq!(svc.vfs().take_warnings().len(), 0);
}
