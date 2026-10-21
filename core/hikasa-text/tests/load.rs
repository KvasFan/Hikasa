//! `load_document` (§1.5.2): reading through a `Vfs`, origin and raw bytes, the size limit before reading,
//! VFS failures (fault injection), cancellation, progress, and the External conversion with a fake runner.

mod common;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::ProgressLog;
use hikasa_core::{
    CancelToken, CaseSensitivity, Classify, Ctl, ErrorKind, FileTime, LocalPath, Phase, Side, Spec,
};
use hikasa_formats::{
    CompiledFormat, Conversion, ConvertError, ExternalConversion, FileFormat, FilenameEncoding,
    FormatId, FormatKind, ProcessRunner, RegexLimits,
};
use hikasa_text::{LineIdx, LoadOptions, TextError, load_document};
use hikasa_vfs::{Fault, MemMeta, MemoryVfs, VfsOp};

fn spec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(path)).unwrap())
}

fn vfs() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_file(
        "/w/a.txt",
        b"alpha\r\nbeta\n".to_vec(),
        MemMeta {
            mtime: FileTime::from_unix(1_700_000_000, 0),
            ..MemMeta::default()
        },
    )
    .unwrap();
    vfs.insert_dir("/w/dir", MemMeta::default()).unwrap();
    vfs
}

#[test]
fn loads_lines_origin_and_raw_bytes() {
    let vfs = vfs();
    let log = ProgressLog::default();
    let cancel = CancelToken::never();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &log,
    };
    let ee = CompiledFormat::everything_else();
    let (doc, raw) = load_document(
        &vfs,
        &spec("/w/a.txt"),
        &ee,
        &LoadOptions::default(),
        None,
        &ctl,
    )
    .unwrap();
    assert_eq!(raw, b"alpha\r\nbeta\n");
    assert_eq!(doc.line_count(), 2);
    assert_eq!(doc.line(LineIdx(0)), "alpha");
    assert_eq!(doc.origin().spec, Some(spec("/w/a.txt")));
    assert_eq!(doc.origin().size, 12);
    assert_eq!(doc.origin().mtime, FileTime::from_unix(1_700_000_000, 0));
    assert!(!doc.converted());
    let reports = log.0.into_inner().unwrap();
    let phases: Vec<Phase> = reports.iter().map(|p| p.phase).collect();
    assert!(phases.contains(&Phase::Reading) && phases.contains(&Phase::Decoding));
    let reading: Vec<u64> = reports
        .iter()
        .filter(|p| p.phase == Phase::Reading)
        .map(|p| p.done)
        .collect();
    assert!(reading.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(reading.last(), Some(&12));
}

#[test]
fn errors_name_the_failure() {
    let vfs = vfs();
    let ee = CompiledFormat::everything_else();
    let ctl = Ctl::detached();
    let opts = LoadOptions::default();
    let err = load_document(&vfs, &spec("/w/missing.txt"), &ee, &opts, None, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.to_string().starts_with("left input: "), "{err}");
    let err = err.with_side(Side::Right);
    assert!(err.to_string().starts_with("right input: "), "{err}");
    let err = load_document(&vfs, &spec("/w/dir"), &ee, &opts, None, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::IsADirectory);
    // The size limit is checked before reading.
    vfs.insert_file("/w/big.txt", vec![b'x'; 10_000], MemMeta::default())
        .unwrap();
    let small = LoadOptions {
        max_bytes: 5000,
        ..LoadOptions::default()
    };
    let err = load_document(&vfs, &spec("/w/big.txt"), &ee, &small, None, &ctl).unwrap_err();
    assert!(
        matches!(err, TextError::TooLarge { limit: 5000, .. }),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::TooLarge);
    // A read failure (fault injection) keeps its kind.
    vfs.inject(Fault {
        path: "/w/a.txt".into(),
        op: VfsOp::Read,
        kind: ErrorKind::PermissionDenied,
    });
    vfs.inject(Fault {
        path: "/w/a.txt".into(),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    let err = load_document(&vfs, &spec("/w/a.txt"), &ee, &opts, None, &ctl).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
}

#[test]
fn cancellation_is_an_error_state_of_the_call_only() {
    let vfs = vfs();
    let cancel = CancelToken::new();
    cancel.cancel();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &hikasa_core::NullProgress,
    };
    let err = load_document(
        &vfs,
        &spec("/w/a.txt"),
        &CompiledFormat::everything_else(),
        &LoadOptions::default(),
        None,
        &ctl,
    )
    .unwrap_err();
    assert!(matches!(err, TextError::Cancelled));
}

/// A sink that cancels the token when the `decoding` phase starts (the reviewers' probe).
struct CancelAtDecoding<'c>(&'c CancelToken);

impl hikasa_core::ProgressSink for CancelAtDecoding<'_> {
    fn report(&self, progress: hikasa_core::Progress) {
        if progress.phase == Phase::Decoding && progress.done == 0 {
            self.0.cancel();
        }
    }
}

/// Decision 3 (§5.3): a cancellation that lands while the document is decoded and built is
/// `Err(Cancelled)`, never `Ok`, whatever the encoding path (valid UTF-8, legacy decoding, UTF-16).
#[test]
fn a_cancel_during_decoding_is_never_answered_with_ok() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let mut latin1 = "caf\u{e9} cr\u{e8}me\n".repeat(50_000).into_bytes();
    for b in latin1.iter_mut() {
        if *b == 0xC3 {
            *b = b'e';
        }
    }
    let utf16: Vec<u8> = "hello\n"
        .repeat(100_000)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    vfs.insert_file(
        "/w/u8.txt",
        "x\n".repeat(400_000).into_bytes(),
        MemMeta::default(),
    )
    .unwrap();
    vfs.insert_file("/w/l1.txt", latin1, MemMeta::default())
        .unwrap();
    vfs.insert_file("/w/u16.txt", utf16, MemMeta::default())
        .unwrap();
    for path in ["/w/u8.txt", "/w/l1.txt", "/w/u16.txt"] {
        let cancel = CancelToken::new();
        let sink = CancelAtDecoding(&cancel);
        let ctl = Ctl {
            cancel: &cancel,
            progress: &sink,
        };
        let result = load_document(
            &vfs,
            &spec(path),
            &CompiledFormat::everything_else(),
            &LoadOptions::default(),
            None,
            &ctl,
        );
        assert!(
            matches!(result, Err(TextError::Cancelled)),
            "{path}: {result:?}"
        );
    }
}

/// Upper-cases `%s` into `%t`.
struct Upper;

impl ProcessRunner for Upper {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        cancel.check()?;
        let source = std::fs::read(&argv[1])?;
        std::fs::write(&argv[2], source.to_ascii_uppercase())?;
        Ok(0)
    }
}

fn external_format() -> CompiledFormat {
    let mut f = FileFormat::new(FormatId::new("upper").unwrap(), "Upper", FormatKind::Text);
    f.conversion = Conversion::External(ExternalConversion {
        load: "upper %s %t".into(),
        save: None,
        filename_encoding: FilenameEncoding::Unicode,
        timeout_secs: 5,
    });
    CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap()
}

#[test]
fn external_conversion_runs_with_the_injected_runner() {
    let vfs = vfs();
    let temp = tempfile::tempdir().unwrap();
    let format = external_format();
    let (doc, raw) = load_document(
        &vfs,
        &spec("/w/a.txt"),
        &format,
        &LoadOptions::default(),
        Some((temp.path(), &Upper)),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(raw, b"alpha\r\nbeta\n", "the raw bytes are the source's");
    assert!(doc.converted());
    assert_eq!(doc.line(LineIdx(1)), "BETA");
    assert_eq!(doc.origin().size, 12);
    // Without a runner the load fails instead of silently comparing unconverted text.
    let err = load_document(
        &vfs,
        &spec("/w/a.txt"),
        &format,
        &LoadOptions::default(),
        None,
        &Ctl::detached(),
    )
    .unwrap_err();
    assert!(matches!(err, TextError::Convert(_)), "{err}");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    // The temporary directory is left empty.
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
