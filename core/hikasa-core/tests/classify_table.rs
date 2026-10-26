//! T3 (§0.2, §5.5): every error type of hikasa-core implements `Classify`, and every variant maps to its
//! documented `ErrorKind`. Each variant is also produced from real input where an input produces it, so the
//! table cannot drift from the parsers.

use std::path::PathBuf;

use hikasa_core::error::{Classify, ErrorKind, ErrorReport, InvariantViolation};
use hikasa_core::exit::{ExitCode, UnknownExitCode};
use hikasa_core::mask::{CaseSensitivity, Mask, MaskError, MaskOptions};
use hikasa_core::path::{RelPath, RelPathError};
use hikasa_core::schema::{SchemaError, check_schema_version};
use hikasa_core::spec::{PathStyle, Spec, SpecContext, SpecError};
use hikasa_core::time::{FileTime, TimeParseError};
use hikasa_core::{Cancelled, ReadError};
use pretty_assertions::assert_eq;

/// Compile-time check: the type is a crate error (`std::error::Error + Classify`), so `ErrorReport::capture`
/// accepts it.
fn report<E: std::error::Error + Classify>(err: &E) -> ErrorReport {
    ErrorReport::capture(err, None)
}

fn spec_error(input: &str, style: PathStyle, cwd: Option<&str>, home: Option<&str>) -> SpecError {
    let ctx = SpecContext {
        style,
        cwd: cwd.map(PathBuf::from),
        home: home.map(PathBuf::from),
    };
    Spec::parse(input, &ctx).unwrap_err()
}

#[test]
fn spec_error_kinds() {
    use PathStyle::{Unix, Windows};
    let cases: Vec<(SpecError, SpecError, ErrorKind)> = vec![
        (
            spec_error("  ", Unix, None, None),
            SpecError::Empty,
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("a\0b", Unix, None, None),
            SpecError::Nul,
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("x", Unix, None, None),
            SpecError::RelativeWithoutCwd { path: "x".into() },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("~", Unix, None, None),
            SpecError::HomeUnknown,
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("~bob", Unix, None, Some("/home/u")),
            SpecError::TildeUser {
                input: "~bob".into(),
            },
            ErrorKind::Unsupported,
        ),
        (
            spec_error("foo://x", Unix, None, None),
            SpecError::UnknownScheme {
                scheme: "foo".into(),
            },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("ftp://h:0/", Unix, None, None),
            SpecError::MalformedUrl {
                input: "ftp://h:0/".into(),
                reason: "port out of range (1-65535)",
            },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("profile:", Unix, None, None),
            SpecError::MalformedProfile {
                input: "profile:".into(),
            },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("D:x", Windows, Some(r"C:\w"), None),
            SpecError::DriveRelative { path: "D:x".into() },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error(r"\\.\pipe\x", Windows, None, None),
            SpecError::DevicePath {
                path: r"\\.\pipe\x".into(),
            },
            ErrorKind::Unsupported,
        ),
        (
            spec_error(r"\\srv", Windows, None, None),
            SpecError::UncWithoutShare {
                path: r"\\srv".into(),
            },
            ErrorKind::InvalidInput,
        ),
        (
            spec_error("x", Unix, Some("relative"), None),
            SpecError::NotNormalized {
                path: "relative".into(),
            },
            ErrorKind::InvalidInput,
        ),
    ];
    for (produced, expected, kind) in cases {
        assert_eq!(produced, expected);
        assert_eq!(produced.kind(), kind, "{produced:?}");
        assert_eq!(report(&produced).kind, kind);
    }
    #[cfg(unix)]
    {
        let base = hikasa_core::spec::LocalPath::new(PathBuf::from("/a")).unwrap();
        let err = base.join_name(std::ffi::OsStr::new("x/y")).unwrap_err();
        assert_eq!(err, SpecError::InvalidComponent { name: "x/y".into() });
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = hikasa_core::spec::LocalPath::new(PathBuf::from("/a//b")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }
    assert_eq!(
        SpecError::InvalidComponent { name: "..".into() }.kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn mask_error_kinds() {
    let opts = MaskOptions {
        case: CaseSensitivity::Insensitive,
    };
    for (item, expected) in [
        (
            "a[b",
            MaskError::UnclosedBracket {
                mask: "a[b".into(),
                at: 1,
            },
        ),
        (
            "[z-a]",
            MaskError::InvalidRange {
                mask: "[z-a]".into(),
                at: 1,
                lo: 'z',
                hi: 'a',
            },
        ),
        (
            "a\\\\b",
            MaskError::EmptyComponent {
                mask: "a\\\\b".into(),
            },
        ),
        (
            "x\\..\\y",
            MaskError::DotComponent {
                mask: "x\\..\\y".into(),
            },
        ),
    ] {
        let produced = Mask::parse(item, opts).unwrap_err();
        assert_eq!(produced, expected);
        assert_eq!(produced.kind(), ErrorKind::InvalidInput, "{item:?}");
        assert_eq!(report(&produced).kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn other_error_kinds() {
    let time: TimeParseError = FileTime::parse("yesterday").unwrap_err();
    assert_eq!(time.kind(), ErrorKind::InvalidInput);
    assert_eq!(report(&time).kind, ErrorKind::InvalidInput);

    let relpath: RelPathError = RelPath::parse("a/../b").unwrap_err();
    assert_eq!(relpath.kind(), ErrorKind::InvalidInput);
    assert_eq!(report(&relpath).kind, ErrorKind::InvalidInput);

    let exit: UnknownExitCode = ExitCode::try_from(42u8).unwrap_err();
    assert_eq!(exit.kind(), ErrorKind::InvalidData);
    assert_eq!(report(&exit).kind, ErrorKind::InvalidData);

    assert_eq!(Cancelled.kind(), ErrorKind::Cancelled);
    assert_eq!(report(&Cancelled).kind, ErrorKind::Cancelled);

    let schema: SchemaError = check_schema_version(3, 2).unwrap_err();
    assert_eq!(schema.kind(), ErrorKind::Unsupported);
    assert_eq!(report(&schema).kind, ErrorKind::Unsupported);

    let invariant = InvariantViolation {
        id: "INV-TEST-1",
        detail: "x".into(),
    };
    assert_eq!(invariant.kind(), ErrorKind::Internal);
    assert_eq!(report(&invariant).kind, ErrorKind::Internal);

    for (err, kind) in [
        (ReadError::TooLarge { limit: 1 }, ErrorKind::TooLarge),
        (ReadError::Cancelled, ErrorKind::Cancelled),
        (ReadError::from(Cancelled), ErrorKind::Cancelled),
        (
            ReadError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)),
            ErrorKind::NotFound,
        ),
        (
            ReadError::Io(std::io::Error::other(Cancelled)),
            ErrorKind::Cancelled,
        ),
        (
            ReadError::Io(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            ErrorKind::Io,
        ),
    ] {
        assert_eq!(err.kind(), kind, "{err:?}");
        assert_eq!(report(&err).kind, kind);
    }

    let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(Classify::kind(&io), ErrorKind::PermissionDenied);
    let captured = report(&io);
    assert_eq!(captured.kind(), ErrorKind::PermissionDenied);
}
