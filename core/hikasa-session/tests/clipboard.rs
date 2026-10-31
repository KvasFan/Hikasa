//! M2 W3 clipboard and `vfs` write operations through the Service (contract M2 §1.3.6, §1.3.7):
//! A3.9 (clipboard content, errors, lazy capture, snapshot, PNG), A3.10 (the GUI handover of `clipboard://`,
//! D-101) and the Service side of A3.11/A3.12 (plans, item failures, nothing written by a dry run).

mod common;

use std::ffi::OsString;
use std::sync::Arc;

use common::{CloseMode, FakeGui, job, os, write};
use hikasa_core::{Classify, ErrorKind, ExitCode, JobKind};
use hikasa_session::api::{
    ApiOp, GuiOpenRequest, QuickMethod, QuickRequest, TextCompareRequest, TouchTime, VfsOpRequest,
};
use hikasa_session::clipboard::{
    ClipboardContent, FixedClipboard, MAX_IMAGE_BYTES, clipboard_blob, encode_png_rgba,
};
use hikasa_session::ipc::{IpcServer, OpenOptions, Request, ViewKind};
use hikasa_session::{ApiError, Service, ServiceInit};
use hikasa_vfs::FollowLinks;
use hikasa_vfs::ops::{CopyOptions, RemoveOptions};

fn service_with_clipboard(root: &std::path::Path, clipboard: Arc<FixedClipboard>) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(2),
        clipboard,
        ..ServiceInit::new(
            common::facts(root),
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

fn clip() -> OsString {
    OsString::from("clipboard://")
}

fn quick(service: &Service, left: OsString, right: OsString) -> Result<ExitCode, ApiError> {
    let req = QuickRequest {
        left,
        right,
        method: QuickMethod::Binary,
        ..QuickRequest::default()
    };
    service
        .quick(&req, &job(service, JobKind::QuickCompare))
        .map(|r| r.exit_code)
}

// ---- A3.9 ---------------------------------------------------------------------------------------------------

#[test]
fn a3_9_text_through_quick_text_compare_cat_stat_and_cp() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text("hello\n"));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let w = tmp.path().join("work");
    let same = write(&w, "same.txt", b"hello\n");
    let other = write(&w, "other.txt", b"bye\n");
    assert_eq!(
        quick(&service, clip(), os(&same)).unwrap(),
        ExitCode::BinarySame
    );
    assert_eq!(
        quick(&service, clip(), os(&other)).unwrap(),
        ExitCode::BinaryDifferent
    );
    let text = service
        .text_compare(
            &TextCompareRequest {
                left: clip(),
                right: os(&same),
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert_eq!(text.exit_code, ExitCode::BinarySame);
    let mut out = Vec::new();
    service
        .vfs_cat(&clip(), &mut out, &job(&service, JobKind::VfsRead))
        .unwrap();
    assert_eq!(out, b"hello\n");
    let stat = service.vfs_stat(&clip(), FollowLinks::No).unwrap();
    assert_eq!(stat.entry.size, 6);
    let target = w.join("from-clipboard.txt");
    let dto = service
        .vfs_run(
            &VfsOpRequest::Copy {
                sources: vec![clip()],
                target: os(&target),
                options: CopyOptions::default(),
            },
            &job(&service, JobKind::VfsWrite),
        )
        .unwrap();
    assert!(dto.ok(), "{dto:?}");
    assert_eq!(std::fs::read(&target).unwrap(), b"hello\n");
    assert_eq!(fixed.reads(), 1, "captured once per service");
}

#[test]
fn a3_9_cp_clipboard_into_a_folder_needs_a_file_name() {
    let tmp = common::short_tempdir();
    let service = service_with_clipboard(tmp.path(), Arc::new(FixedClipboard::text("x")));
    let w = tmp.path().join("work");
    std::fs::create_dir_all(&w).unwrap();
    let err = service
        .vfs_run(
            &VfsOpRequest::Copy {
                sources: vec![clip()],
                target: os(&w),
                options: CopyOptions::default(),
            },
            &job(&service, JobKind::VfsWrite),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("needs a file name"), "{err}");
}

#[test]
fn a3_9_empty_unavailable_and_failing_clipboards() {
    for (fixed, kind) in [
        (FixedClipboard::empty(), ErrorKind::NotFound),
        (
            FixedClipboard::unavailable("headless"),
            ErrorKind::Unsupported,
        ),
        (FixedClipboard::failing("broken pipe"), ErrorKind::Io),
    ] {
        let tmp = common::short_tempdir();
        let fixed = Arc::new(fixed);
        let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
        let w = tmp.path().join("work");
        let a = write(&w, "a.txt", b"a\n");
        let err = quick(&service, clip(), os(&a)).unwrap_err();
        assert!(matches!(err, ApiError::LoadFailed { .. }), "{err:?}");
        assert_eq!(err.kind(), kind);
        assert_eq!(err.exit_code(ApiOp::Quick), ExitCode::UnknownError);
        let err = service
            .text_compare(
                &TextCompareRequest {
                    left: os(&a),
                    right: clip(),
                    ..TextCompareRequest::default()
                },
                &job(&service, JobKind::TextCompare),
                None,
            )
            .unwrap_err();
        assert_eq!(err.kind(), kind, "{err:?}");
        assert_eq!(err.exit_code(ApiOp::TextCompare), ExitCode::UnknownError);
        let mut out = Vec::new();
        let err = service
            .vfs_cat(&clip(), &mut out, &job(&service, JobKind::VfsRead))
            .unwrap_err();
        assert_eq!(err.kind(), kind);
        assert_eq!(err.exit_code(ApiOp::Vfs), ExitCode::UnknownError);
        let err = service.vfs_stat(&clip(), FollowLinks::No).unwrap_err();
        assert_eq!(err.kind(), kind);
        let dto = service
            .vfs_run(
                &VfsOpRequest::Copy {
                    sources: vec![clip()],
                    target: os(&w.join("x.txt")),
                    options: CopyOptions::default(),
                },
                &job(&service, JobKind::VfsWrite),
            )
            .unwrap();
        assert!(!dto.ok());
        assert_eq!(dto.items[0].error.as_ref().unwrap().kind, kind);
        assert_eq!(fixed.reads(), 1, "a failure is cached for the service");
        assert!(service.read_clipboard().is_err());
        assert_eq!(fixed.reads(), 2, "read_clipboard retries");
    }
}

#[test]
fn a3_9_the_provider_is_called_only_for_clipboard_specs_and_once() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text("same\n"));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let b = write(&w, "b.txt", b"b\n");
    quick(&service, os(&a), os(&b)).unwrap();
    service
        .vfs_ls(&os(&w), FollowLinks::No, &job(&service, JobKind::VfsList))
        .unwrap();
    service
        .vfs_run(
            &VfsOpRequest::Mkdir {
                specs: vec![os(&w.join("d"))],
                parents: false,
            },
            &job(&service, JobKind::VfsWrite),
        )
        .unwrap();
    assert_eq!(fixed.reads(), 0);
    assert_eq!(
        quick(&service, clip(), clip()).unwrap(),
        ExitCode::BinarySame
    );
    assert_eq!(fixed.reads(), 1, "both sides see one snapshot");
}

#[test]
fn a3_9_read_clipboard_replaces_the_snapshot() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text("first"));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let cat = |service: &Service| {
        let mut out = Vec::new();
        service
            .vfs_cat(&clip(), &mut out, &job(service, JobKind::VfsRead))
            .unwrap();
        out
    };
    assert_eq!(cat(&service), b"first");
    fixed.set_text("second");
    assert_eq!(cat(&service), b"first", "the snapshot is kept");
    let dto = service.read_clipboard().unwrap();
    assert_eq!((dto.kind, dto.size, dto.name.as_deref()), ("text", 6, None));
    assert_eq!(cat(&service), b"second");
    assert_eq!(fixed.reads(), 2);
}

/// A minimal PNG reader: checks the signature, every chunk CRC, the zlib header, the stored blocks and the
/// Adler-32, and returns (width, height, colour type, the unfiltered RGBA pixels).
fn decode_png(png: &[u8]) -> (u32, u32, u8, Vec<u8>) {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut at = 8;
    let (mut width, mut height, mut colour) = (0, 0, 0);
    let mut idat = Vec::new();
    let mut kinds = Vec::new();
    while at < png.len() {
        let len = be(&png[at..]) as usize;
        let kind = &png[at + 4..at + 8];
        let data = &png[at + 8..at + 8 + len];
        let crc = be(&png[at + 8 + len..]);
        assert_eq!(crc, crc32(&png[at + 4..at + 8 + len]), "CRC of {kind:?}");
        kinds.push(String::from_utf8_lossy(kind).into_owned());
        match kind {
            b"IHDR" => {
                width = be(data);
                height = be(&data[4..]);
                assert_eq!(data[8], 8, "bit depth");
                colour = data[9];
            }
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        at += 12 + len;
    }
    assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
    assert_eq!(&idat[..2], [0x78, 0x01]);
    assert_eq!((u16::from(idat[0]) << 8 | u16::from(idat[1])) % 31, 0);
    let mut raw = Vec::new();
    let mut pos = 2;
    loop {
        let last = idat[pos] & 1;
        assert_eq!(idat[pos] >> 1, 0, "stored block");
        let len = u16::from_le_bytes([idat[pos + 1], idat[pos + 2]]);
        let nlen = u16::from_le_bytes([idat[pos + 3], idat[pos + 4]]);
        assert_eq!(len, !nlen);
        raw.extend_from_slice(&idat[pos + 5..pos + 5 + len as usize]);
        pos += 5 + len as usize;
        if last == 1 {
            break;
        }
    }
    let adler = be(&idat[pos..]);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    assert_eq!(adler, (b << 16) | a);
    let row = width as usize * 4 + 1;
    let mut pixels = Vec::new();
    for line in raw.chunks(row) {
        assert_eq!(line[0], 0, "filter 0");
        pixels.extend_from_slice(&line[1..]);
    }
    (width, height, colour, pixels)
}

#[test]
fn a3_9_images_are_valid_pngs_named_clipboard_png() {
    let (w, h) = (3u32, 2u32);
    let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 256) as u8).collect();
    let png = encode_png_rgba(w, h, &rgba).unwrap();
    assert_eq!(decode_png(&png), (w, h, 6, rgba.clone()));
    assert_eq!(png, encode_png_rgba(w, h, &rgba).unwrap(), "deterministic");
    // Several stored blocks (more than 65,535 raw bytes).
    let (bw, bh) = (200u32, 100u32);
    let big: Vec<u8> = (0..bw * bh * 4).map(|i| (i % 251) as u8).collect();
    assert_eq!(decode_png(&encode_png_rgba(bw, bh, &big).unwrap()).3, big);
    // Through the Service: the blob is named clipboard.png.
    let tmp = common::short_tempdir();
    let service = service_with_clipboard(
        tmp.path(),
        Arc::new(FixedClipboard::image(w, h, rgba.clone())),
    );
    let dto = service.read_clipboard().unwrap();
    assert_eq!(
        (dto.kind, dto.name.as_deref()),
        ("image", Some("clipboard.png"))
    );
    let mut out = Vec::new();
    service
        .vfs_cat(&clip(), &mut out, &job(&service, JobKind::VfsRead))
        .unwrap();
    assert_eq!(decode_png(&out).3, rgba);
    let blob = clipboard_blob(
        ClipboardContent::Text("t".into()),
        hikasa_core::FileTime::from_unix(0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(blob.name, None, "text has no name hint");
}

#[test]
fn a3_9_more_than_a_gibibyte_of_pixels_is_too_large() {
    let err = encode_png_rgba(65_536, 4_097, &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge);
    assert!(u64::from(65_536u32) * 4_097 * 4 > MAX_IMAGE_BYTES);
    let err = encode_png_rgba(2, 2, &[0; 3]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

// ---- A3.10 --------------------------------------------------------------------------------------------------

#[test]
fn a3_10_gui_open_sends_clipboard_verbatim_and_never_reads_it() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text("x"));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = IpcServer::bind(&service.endpoint().unwrap())
        .unwrap()
        .spawn(Arc::clone(&gui) as Arc<dyn hikasa_session::ipc::IpcHandler>)
        .unwrap();
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let req = GuiOpenRequest {
        specs: vec![clip(), os(&a)],
        ..GuiOpenRequest::default()
    };
    service.gui_open(&req, None).unwrap();
    let sent = gui
        .requests
        .lock()
        .iter()
        .find_map(|r| match r {
            Request::OpenView { kind, specs, .. } => Some((*kind, specs.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(sent.0, ViewKind::TextCompare);
    assert_eq!(sent.1[0], "clipboard://");
    assert_eq!(
        fixed.reads(),
        0,
        "the CLI never reads the clipboard for the GUI"
    );
    server.shutdown().unwrap();
}

#[test]
fn a3_10_the_handler_path_reads_the_injected_provider() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text("x"));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let w = tmp.path().join("work");
    let a = os(&write(&w, "a.txt", b"a\n")).into_string().unwrap();
    let specs = vec!["clipboard://".to_owned(), a.clone()];
    let options = OpenOptions::default();
    service
        .open_view_request(ViewKind::TextCompare, &specs, &options)
        .unwrap();
    assert_eq!(
        fixed.reads(),
        0,
        "open_view_request accepts it without reading"
    );
    service
        .open_view_from_ipc(ViewKind::TextCompare, &specs, &options)
        .unwrap();
    assert_eq!(fixed.reads(), 1);
    service
        .open_view_from_ipc(ViewKind::TextCompare, &specs, &options)
        .unwrap();
    assert_eq!(fixed.reads(), 2, "every open re-reads the GUI's clipboard");
    // `-` is still refused over IPC.
    let err = service
        .open_view_request(
            ViewKind::TextCompare,
            &["-".to_owned(), a.clone()],
            &options,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)));
    // A provider error is the handler's error (answered `invalid_request`).
    let tmp2 = common::short_tempdir();
    let none = service_with_clipboard(
        tmp2.path(),
        Arc::new(FixedClipboard::unavailable("headless")),
    );
    let err = none
        .open_view_from_ipc(ViewKind::TextCompare, &specs, &options)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[test]
fn a3_10_rules_files_are_still_refused_as_clipboard() {
    let tmp = common::short_tempdir();
    let service = service_with_clipboard(tmp.path(), Arc::new(FixedClipboard::text("x")));
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a\n");
    let req = GuiOpenRequest {
        specs: vec![os(&a), os(&a)],
        text_rules_file: Some(clip()),
        ..GuiOpenRequest::default()
    };
    assert!(matches!(
        service.resolve_open(&req),
        Err(ApiError::InvalidArgument(_))
    ));
}

// ---- A3.11 / A3.12 (Service side) ---------------------------------------------------------------------------

/// Every path below `root`, with file contents (a tree snapshot).
fn snapshot(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.push(path.clone());
                out.push((path.display().to_string(), None));
            } else {
                out.push((path.display().to_string(), std::fs::read(&path).ok()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a3_12_dry_runs_and_clipboard_reads_write_nothing() {
    let tmp = common::short_tempdir();
    let service = service_with_clipboard(tmp.path(), Arc::new(FixedClipboard::text("x")));
    let w = tmp.path().join("work");
    write(&w, "src/a.txt", b"a");
    write(&w, "src/sub/b.txt", b"b");
    let before = snapshot(tmp.path());
    let requests = [
        VfsOpRequest::Copy {
            sources: vec![os(&w.join("src"))],
            target: os(&w.join("dst")),
            options: CopyOptions {
                recursive: true,
                ..CopyOptions::default()
            },
        },
        VfsOpRequest::Remove {
            specs: vec![os(&w.join("src"))],
            options: RemoveOptions {
                recursive: true,
                trash: false,
            },
        },
        VfsOpRequest::Mkdir {
            specs: vec![os(&w.join("new/deep"))],
            parents: true,
        },
        VfsOpRequest::Touch {
            specs: vec![os(&w.join("c.txt"))],
            time: TouchTime::Now,
            created: None,
            create: true,
        },
    ];
    for req in &requests {
        let dto = service
            .vfs_plan(req, &job(&service, JobKind::VfsWrite))
            .unwrap();
        assert!(dto.dry_run);
        assert!(
            dto.items
                .iter()
                .all(|i| i.status == hikasa_vfs::ops::ItemStatus::Planned)
        );
    }
    service.read_clipboard().unwrap();
    let mut out = Vec::new();
    service
        .vfs_cat(&clip(), &mut out, &job(&service, JobKind::VfsRead))
        .unwrap();
    assert_eq!(snapshot(tmp.path()), before);
}

// ---- W3 review fixes (§1.3.12) -----------------------------------------------------------------------------

#[test]
fn fix_empty_text_is_an_empty_clipboard() {
    let tmp = common::short_tempdir();
    let fixed = Arc::new(FixedClipboard::text(""));
    let service = service_with_clipboard(tmp.path(), Arc::clone(&fixed));
    let a = write(&tmp.path().join("work"), "a.txt", b"a\n");
    let err = quick(&service, clip(), os(&a)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err:?}");
    let err = service.vfs_stat(&clip(), FollowLinks::No).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err:?}");
    let err = service.read_clipboard().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err:?}");
    let err = clipboard_blob(
        ClipboardContent::Text(String::new()),
        hikasa_core::FileTime::from_unix(0, 0).unwrap(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn fix_clipboard_file_above_the_text_limit_is_too_large() {
    use hikasa_session::clipboard::{ClipboardError, ClipboardProvider, MAX_TEXT_BYTES};
    use hikasa_session::host::FileClipboard;
    let tmp = common::short_tempdir();
    let path = tmp.path().join("huge.txt");
    // Sparse: nothing is written or read.
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_TEXT_BYTES + 1)
        .unwrap();
    let provider = FileClipboard(path.clone());
    assert!(matches!(
        provider.read(),
        Err(ClipboardError::TooLarge(limit)) if limit == MAX_TEXT_BYTES
    ));
    // Through the Service: kind `too_large` (exit 100).
    let init = ServiceInit {
        config_dir: Some(tmp.path().join("config")),
        clipboard: Arc::new(provider),
        ..ServiceInit::new(
            common::facts(tmp.path()),
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let err = service.vfs_stat(&clip(), FollowLinks::No).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err:?}");
    assert_eq!(err.exit_code(ApiOp::Vfs), ExitCode::UnknownError);
    // At the limit's edge a small file is fine.
    std::fs::write(&path, b"small").unwrap();
    assert_eq!(
        FileClipboard(path).read().unwrap(),
        ClipboardContent::Text("small".into())
    );
}

#[test]
fn fix_streamed_png_blocks_end_exactly_at_block_boundaries() {
    // Rows of 4 * 64 + 1 = 257 bytes: 255 rows fill one stored block exactly, 510 rows two.
    for height in [1u32, 254, 255, 256, 510, 511] {
        let rgba: Vec<u8> = (0..64 * height * 4).map(|i| (i % 253) as u8).collect();
        let png = encode_png_rgba(64, height, &rgba).unwrap();
        assert_eq!(decode_png(&png), (64, height, 6, rgba), "height {height}");
    }
}
