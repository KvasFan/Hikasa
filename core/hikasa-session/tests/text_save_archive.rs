//! M2 W10b acceptance A10.16 (D-276, contract §1.10.14 *Archives*), on disk through the Service: a member of a zip,
//! of a tar.gz and of a zip inside a tar is edited and saved; the archive is rebuilt once (one backup of the archive,
//! holding the archive as it was, also for two members saved in one plan), every other member stays byte-identical
//! (the raw records, as W5's archive tests compare them); a member of a gz stream is `ArchiveReadOnly` and its save
//! is refused before planning; a rebuild that finds the archive changed since the plan fails the item.

mod common;

use std::io::{Read as _, Write as _};
use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::{ErrorKind, ExitCode, JobKind, Side};
use hikasa_session::api::{
    LiveOptions, LiveTextView, TextCompareRequest, apply_save_report, save_exit_code,
};
use hikasa_session::{ApiError, Service};
use hikasa_text::save::{SaveRequest, UnmappablePolicy};
use hikasa_text::{EditBlock, EditRequest, EditableDocument, GroupHint, TextPos};
use hikasa_vfs::ops::ItemStatus;

// ---- archives made by the test -----------------------------------------------------------------------------------

/// A zip of `files`, the first stored, the others deflated.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (i, (name, data)) in files.iter().enumerate() {
        let method = if i == 0 {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The raw local record (header and data, as stored) of `name` in `zip`.
fn zip_record(zip: &[u8], name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let file = archive.by_name_raw(name.as_bytes()).unwrap();
    let start = usize::try_from(file.header_start()).unwrap();
    let end = usize::try_from(file.data_start().unwrap() + file.compressed_size()).unwrap();
    zip[start..end].to_vec()
}

/// The content of `name` in `zip`.
fn zip_member(zip: &[u8], name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut out = Vec::new();
    file.read_to_end(&mut out).unwrap();
    out
}

/// One ustar header block for a regular file.
fn tar_header(name: &str, size: usize, mtime: u64) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    let octal = |field: &mut [u8], value: u64| {
        let text = format!("{value:0width$o}", width = field.len() - 1);
        field[..text.len()].copy_from_slice(text.as_bytes());
    };
    octal(&mut h[100..108], 0o644);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size as u64);
    octal(&mut h[136..148], mtime);
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    let text = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(text.as_bytes());
    h
}

/// A ustar archive of `files` (each padded to 512 bytes), with the two zero blocks at its end.
fn tar_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        out.extend_from_slice(&tar_header(name, data.len(), 1_700_000_000));
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

/// The members of a plain tar: name → (header, data).
fn tar_members(tar: &[u8]) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = String::from_utf8_lossy(&header[..name_end]).into_owned();
        let size_text = String::from_utf8_lossy(&header[124..136]);
        let size = u64::from_str_radix(size_text.trim_matches(|c: char| c == '\0' || c == ' '), 8)
            .unwrap() as usize;
        let data = tar[at + 512..at + 512 + size].to_vec();
        out.push((name, header.to_vec(), data));
        at += 512 + size.div_ceil(512) * 512;
    }
    out
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

/// A gzip stream of stored deflate blocks.
fn gz_of(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let chunks: Vec<&[u8]> = data.chunks(65_535).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = u16::try_from(chunk.len()).unwrap();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

// ---- helpers -----------------------------------------------------------------------------------------------------

/// Opens two specs live.
fn open(service: &Service, left: &Path, right: &Path) -> LiveTextView {
    let req = TextCompareRequest {
        left: os(left),
        right: os(right),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live(
            &req,
            LiveOptions::default(),
            &job(service, JobKind::TextCompare),
            None,
        )
        .unwrap()
}

fn type_x(doc: &mut EditableDocument) {
    let at = TextPos::new(0, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: "X".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

/// A member's content read through a fresh service (no index of the old archive).
fn cat(root: &Path, spec: &Path) -> Vec<u8> {
    let service = service(root);
    let mut out = Vec::new();
    service
        .vfs_cat(spec.as_os_str(), &mut out, &job(&service, JobKind::VfsRead))
        .unwrap();
    out
}

/// A save of `docs` in place with backups on.
fn save(
    service: &Service,
    docs: &[(Option<Side>, &EditableDocument)],
) -> hikasa_text::save::SaveReport {
    let req = SaveRequest::in_place(
        service.backup_policy_for_save(false).unwrap(),
        UnmappablePolicy::Ask,
    );
    let triples: Vec<_> = docs.iter().map(|(side, doc)| (*side, *doc, &req)).collect();
    let job = job(service, JobKind::TextSave);
    let plan = service.text_save_plan(&triples, &job).unwrap();
    assert!(plan.confirmed, "{:?}", plan.items);
    let only: Vec<&EditableDocument> = docs.iter().map(|(_, doc)| *doc).collect();
    service.text_save_execute(plan, &only, &job).unwrap()
}

// ---- zip ---------------------------------------------------------------------------------------------------------

#[test]
fn a10_16_a_zip_member_is_saved_by_one_rebuild_and_the_others_stay_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let original = zip_of(&[
        ("x.txt", b"one\ntwo\n"),
        ("keep.bin", &[0u8, 1, 2, 3, 255, 254][..]),
        ("sub/y.txt", b"why\nnot\n"),
    ]);
    let zip = write(&w, "a.zip", &original);
    let other = write(&w, "other.txt", b"one\n");
    let mut view = open(&service, &zip.join("x.txt"), &other);
    assert!(view.docs.left.editability().editable());
    type_x(&mut view.docs.left);
    let report = save(&service, &[(Some(Side::Left), &view.docs.left)]);
    assert_eq!(
        save_exit_code(&report),
        ExitCode::Success,
        "{:?}",
        report.op.results
    );
    let rebuilt = std::fs::read(&zip).unwrap();
    assert_eq!(zip_member(&rebuilt, "x.txt"), b"Xone\ntwo\n");
    for name in ["keep.bin", "sub/y.txt"] {
        assert_eq!(
            zip_record(&rebuilt, name),
            zip_record(&original, name),
            "{name}"
        );
    }
    // One backup of the archive, as it was.
    assert_eq!(std::fs::read(w.join("a.zip.bak")).unwrap(), original);
    assert_eq!(cat(root, &zip.join("x.txt")), b"Xone\ntwo\n");
    // The member is the next splice source: a second save splices its unchanged line.
    assert_eq!(apply_save_report(&report, &mut [&mut view.docs.left]), 1);
    assert!(!view.docs.left.is_modified());
}

#[test]
fn a10_16_two_members_of_one_zip_are_one_rebuild_and_one_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let original = zip_of(&[
        ("l.txt", b"left\n"),
        ("r.txt", b"right\n"),
        ("keep.txt", b"kept\n"),
    ]);
    let zip = write(&w, "two.zip", &original);
    let mut view = open(&service, &zip.join("l.txt"), &zip.join("r.txt"));
    type_x(&mut view.docs.left);
    type_x(&mut view.docs.right);
    let req = SaveRequest::in_place(
        service.backup_policy_for_save(false).unwrap(),
        UnmappablePolicy::Ask,
    );
    let job = job(&service, JobKind::TextSave);
    let plan = service
        .text_save_plan(
            &[
                (Some(Side::Left), &view.docs.left, &req),
                (Some(Side::Right), &view.docs.right, &req),
            ],
            &job,
        )
        .unwrap();
    assert_eq!(plan.exec.archive_backups.len(), 1, "{:?}", plan.exec);
    let report = service
        .text_save_execute(plan, &[&view.docs.left, &view.docs.right], &job)
        .unwrap();
    assert!(report.ok(), "{:?}", report.op.results);
    let rebuilt = std::fs::read(&zip).unwrap();
    assert_eq!(zip_member(&rebuilt, "l.txt"), b"Xleft\n");
    assert_eq!(zip_member(&rebuilt, "r.txt"), b"Xright\n");
    assert_eq!(
        zip_record(&rebuilt, "keep.txt"),
        zip_record(&original, "keep.txt")
    );
    assert_eq!(std::fs::read(w.join("two.zip.bak")).unwrap(), original);
    // Without backups none is made.
    let mut view = open(&service, &zip.join("l.txt"), &zip.join("r.txt"));
    type_x(&mut view.docs.left);
    std::fs::remove_file(w.join("two.zip.bak")).unwrap();
    let req = SaveRequest::in_place(
        service.backup_policy_for_save(true).unwrap(),
        UnmappablePolicy::Ask,
    );
    let plan = service
        .text_save_plan(&[(Some(Side::Left), &view.docs.left, &req)], &job)
        .unwrap();
    assert!(plan.exec.archive_backups.is_empty());
    let report = service
        .text_save_execute(plan, &[&view.docs.left], &job)
        .unwrap();
    assert!(report.ok());
    assert!(!w.join("two.zip.bak").exists());
    assert_eq!(
        zip_member(&std::fs::read(&zip).unwrap(), "l.txt"),
        b"XXleft\n"
    );
}

#[test]
fn a10_16_a_rebuild_that_finds_the_archive_changed_fails_the_item() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let zip = write(
        &w,
        "c.zip",
        &zip_of(&[("x.txt", b"one\n"), ("y.txt", b"two\n")]),
    );
    let other = write(&w, "o.txt", b"o\n");
    let mut view = open(&service, &zip.join("x.txt"), &other);
    type_x(&mut view.docs.left);
    let req = SaveRequest::in_place(None, UnmappablePolicy::Ask);
    let job = job(&service, JobKind::TextSave);
    let plan = service
        .text_save_plan(&[(Some(Side::Left), &view.docs.left, &req)], &job)
        .unwrap();
    // Another program rewrites the archive between the plan and the execution.
    let changed = zip_of(&[
        ("x.txt", b"ONE\n"),
        ("y.txt", b"two\n"),
        ("z.txt", b"new\n"),
    ]);
    std::fs::write(&zip, &changed).unwrap();
    let report = service
        .text_save_execute(plan, &[&view.docs.left], &job)
        .unwrap();
    assert_eq!(
        report.status(0),
        Some(ItemStatus::Failed),
        "{:?}",
        report.op.results
    );
    assert_eq!(save_exit_code(&report), ExitCode::UnknownError);
    assert_eq!(std::fs::read(&zip).unwrap(), changed);
    assert_eq!(apply_save_report(&report, &mut [&mut view.docs.left]), 0);
    assert!(view.docs.left.is_modified());
}

// ---- tar.gz, a zip inside a tar, a gz member ----------------------------------------------------------------------

#[test]
fn a10_16_a_tar_gz_member_is_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let tar = tar_of(&[
        ("x.txt", b"one\ntwo\n"),
        ("keep.txt", b"kept\n"),
        ("bin.dat", &[0u8, 9, 8][..]),
    ]);
    let original = gz_of(&tar);
    let tgz = write(&w, "b.tar.gz", &original);
    let other = write(&w, "other.txt", b"one\n");
    let mut view = open(&service, &tgz.join("x.txt"), &other);
    assert!(
        view.docs.left.editability().editable(),
        "{:?}",
        view.docs.left.editability()
    );
    type_x(&mut view.docs.left);
    let report = save(&service, &[(Some(Side::Left), &view.docs.left)]);
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(cat(root, &tgz.join("x.txt")), b"Xone\ntwo\n");
    assert_eq!(cat(root, &tgz.join("keep.txt")), b"kept\n");
    assert_eq!(cat(root, &tgz.join("bin.dat")), [0u8, 9, 8]);
    assert_eq!(std::fs::read(w.join("b.tar.gz.bak")).unwrap(), original);
}

#[test]
fn a10_16_a_member_of_a_zip_inside_a_tar_is_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let inner = zip_of(&[("x.txt", b"one\n"), ("k.txt", b"kept\n")]);
    let original = tar_of(&[
        ("first.txt", b"first\n"),
        ("inner.zip", &inner),
        ("last.txt", b"last\n"),
    ]);
    let tar = write(&w, "c.tar", &original);
    let other = write(&w, "other.txt", b"one\n");
    let member = tar.join("inner.zip").join("x.txt");
    let mut view = open(&service, &member, &other);
    assert!(view.docs.left.editability().editable());
    type_x(&mut view.docs.left);
    let report = save(&service, &[(Some(Side::Left), &view.docs.left)]);
    assert!(report.ok(), "{:?}", report.op.results);
    let rebuilt = std::fs::read(&tar).unwrap();
    let members = tar_members(&rebuilt);
    let old = tar_members(&original);
    let find = |list: &[(String, Vec<u8>, Vec<u8>)], name: &str| {
        list.iter().find(|(n, _, _)| n == name).cloned().unwrap()
    };
    for name in ["first.txt", "last.txt"] {
        let (_, header, data) = find(&members, name);
        let (_, old_header, old_data) = find(&old, name);
        assert_eq!(data, old_data, "{name}");
        assert_eq!(header, old_header, "{name}");
    }
    let (_, _, zip) = find(&members, "inner.zip");
    assert_eq!(zip_member(&zip, "x.txt"), b"Xone\n");
    assert_eq!(zip_record(&zip, "k.txt"), zip_record(&inner, "k.txt"));
    // One backup: of the outermost file, as it was.
    assert_eq!(std::fs::read(w.join("c.tar.bak")).unwrap(), original);
    assert!(!w.join("inner.zip.bak").exists());
}

#[test]
fn a10_16_a_gz_member_is_read_only_and_its_save_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let service = service(root);
    let w = root.join("work");
    let gz = write(&w, "n.txt.gz", &gz_of(b"in gz\n"));
    let other = write(&w, "o.txt", b"o\n");
    let view = open(&service, &gz.join("n.txt"), &other);
    assert_eq!(
        view.docs.left.editability().blocks,
        vec![EditBlock::ArchiveReadOnly]
    );
    let req = SaveRequest::in_place(None, UnmappablePolicy::Ask);
    let err = service
        .text_save_plan(
            &[(Some(Side::Left), &view.docs.left, &req)],
            &job(&service, JobKind::TextSave),
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            ApiError::EditBlocked {
                side: Some(Side::Left),
                block: EditBlock::ArchiveReadOnly
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
}
