//! `ApiError::exit_code` — every error row of §2.21, both `wait` values (D-01, D-02, D-22, D-23, D-35), and the
//! JSON error report of §4.2.

use hikasa_core::{ErrorKind, ExitCode, Side, SpecError};
use hikasa_folder::FolderError;
use hikasa_formats::FormatError;
use hikasa_report::ReportError;
use hikasa_session::api::ApiOp;
use hikasa_session::ipc::IpcError;
use hikasa_session::{ApiError, SessionError};
use hikasa_text::TextError;
use hikasa_vfs::VfsError;

fn not_found(spec: &str) -> VfsError {
    VfsError::NotFound { spec: spec.into() }
}

fn load_failed() -> ApiError {
    ApiError::LoadFailed {
        side: Side::Left,
        spec: "/w/missing.c".into(),
        source: not_found("/w/missing.c"),
    }
}

/// One representative of every error a command can return.
fn every_error() -> Vec<ApiError> {
    vec![
        ApiError::InvalidArgument("x".into()),
        ApiError::Spec(SpecError::Empty),
        load_failed(),
        ApiError::Vfs(VfsError::Unsupported {
            spec: "ftp://h/x".into(),
            what: "ftp specs",
        }),
        ApiError::Format(FormatError::UnknownFormat { name: "x".into() }),
        ApiError::Text(TextError::TooLarge {
            spec: "/x".into(),
            limit: 1,
        }),
        ApiError::Text(TextError::Rules("bad".into())),
        ApiError::Folder(FolderError::InvalidRules("x".into())),
        ApiError::Folder(FolderError::RulesComparatorMissing),
        ApiError::Folder(FolderError::Cancelled),
        ApiError::Report(ReportError::Unsupported {
            layout: "patch",
            format: "html",
        }),
        ApiError::Session(SessionError::NoConfigDir),
        ApiError::Ipc(IpcError::Protocol("x".into())),
        ApiError::Ipc(IpcError::Timeout),
        ApiError::Ipc(IpcError::Remote {
            code: "not_found".into(),
            message: "no view".into(),
        }),
        ApiError::Cancelled,
        ApiError::Internal("panic: boom".into()),
    ]
}

#[test]
fn file_comparisons_and_tools_map_every_error_to_100() {
    for op in [
        ApiOp::Quick,
        ApiOp::TextCompare,
        ApiOp::Format,
        ApiOp::Vfs,
        ApiOp::Other,
    ] {
        for error in every_error() {
            assert_eq!(
                error.exit_code(op),
                ExitCode::UnknownError,
                "{op:?} {error}"
            );
        }
        assert_eq!(
            ApiError::Ipc(IpcError::NotRunning).exit_code(op),
            ExitCode::UnknownError
        );
    }
}

#[test]
fn folder_compare_base_failures_are_107() {
    let op = ApiOp::FolderCompare;
    assert_eq!(load_failed().exit_code(op), ExitCode::LoadFailed);
    assert_eq!(
        ApiError::Folder(FolderError::BaseNotLoadable {
            side: Side::Right,
            spec: "/b".into(),
            source: not_found("/b"),
        })
        .exit_code(op),
        ExitCode::LoadFailed
    );
    assert_eq!(
        ApiError::Folder(FolderError::BaseNotFolder {
            side: Side::Left,
            spec: "/a".into(),
        })
        .exit_code(op),
        ExitCode::LoadFailed
    );
    for error in every_error() {
        if matches!(error, ApiError::LoadFailed { .. }) {
            continue;
        }
        assert_eq!(error.exit_code(op), ExitCode::UnknownError, "{error}");
    }
    // Base failures under another operation are 100.
    assert_eq!(
        ApiError::Folder(FolderError::BaseNotFolder {
            side: Side::Left,
            spec: "/a".into(),
        })
        .exit_code(ApiOp::Quick),
        ExitCode::UnknownError
    );
}

#[test]
fn gui_open_rows() {
    for wait in [false, true] {
        let op = ApiOp::GuiOpen { wait };
        let not_running = if wait {
            ExitCode::CannotWaitForGui
        } else {
            ExitCode::UnknownError
        };
        assert_eq!(
            ApiError::Ipc(IpcError::NotRunning).exit_code(op),
            not_running,
            "wait {wait}"
        );
        assert_eq!(
            ApiError::Ipc(IpcError::LaunchTimeout).exit_code(op),
            ExitCode::CannotWaitForGui
        );
        assert_eq!(
            ApiError::Ipc(IpcError::ConnectionLost).exit_code(op),
            ExitCode::CannotWaitForGui
        );
        assert_eq!(ApiError::GuiNotFound.exit_code(op), ExitCode::GuiNotFound);
        assert_eq!(
            ApiError::GuiNotStartable(std::io::ErrorKind::PermissionDenied.into()).exit_code(op),
            ExitCode::GuiNotFound,
            "an executable that cannot be started is no usable GUI executable (wave-5 review)"
        );
        for other in [
            ApiError::Ipc(IpcError::Timeout),
            ApiError::Ipc(IpcError::IncompatibleVersion {
                client: 1,
                server: 2,
            }),
            ApiError::Ipc(IpcError::Remote {
                code: "unsupported_view".into(),
                message: "x".into(),
            }),
            ApiError::InvalidArgument("clipboard".into()),
            ApiError::Internal("panic".into()),
        ] {
            assert_eq!(other.exit_code(op), ExitCode::UnknownError, "{other}");
        }
    }
}

#[test]
fn gui_control_rows() {
    let op = ApiOp::GuiControl;
    assert_eq!(
        ApiError::Ipc(IpcError::NotRunning).exit_code(op),
        ExitCode::UnknownError
    );
    assert_eq!(
        ApiError::Ipc(IpcError::ConnectionLost).exit_code(op),
        ExitCode::UnknownError
    );
    assert_eq!(
        ApiError::Ipc(IpcError::LaunchTimeout).exit_code(op),
        ExitCode::UnknownError
    );
    for error in every_error() {
        assert_eq!(error.exit_code(op), ExitCode::UnknownError, "{error}");
    }
}

#[test]
fn error_reports_follow_4_2() {
    let report = load_failed().report();
    assert_eq!(report.kind, ErrorKind::NotFound);
    assert_eq!(
        report.message,
        "left input /w/missing.c: /w/missing.c: not found"
    );
    assert_eq!(report.spec.as_deref(), Some("/w/missing.c"));
    assert_eq!(report.causes, vec!["/w/missing.c: not found".to_owned()]);
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert_eq!(
        json,
        "{\n  \"kind\": \"not_found\",\n  \"message\": \"left input /w/missing.c: /w/missing.c: not found\",\n  \
         \"spec\": \"/w/missing.c\",\n  \"causes\": [\n    \"/w/missing.c: not found\"\n  ]\n}"
    );
    let unsupported = ApiError::Vfs(VfsError::Unsupported {
        spec: "ftp://h/x".into(),
        what: "ftp specs",
    })
    .report();
    assert_eq!(unsupported.kind, ErrorKind::Unsupported);
    assert_eq!(unsupported.spec.as_deref(), Some("ftp://h/x"));
    assert_eq!(ApiError::Cancelled.kind(), ErrorKind::Cancelled);
    assert_eq!(ApiError::Internal("x".into()).kind(), ErrorKind::Internal);
    assert_eq!(ApiError::GuiNotFound.kind(), ErrorKind::NotFound);
    assert_eq!(
        ApiError::GuiNotStartable(std::io::ErrorKind::PermissionDenied.into()).kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        ApiError::Ipc(IpcError::Protocol("x".into())).kind(),
        ErrorKind::Protocol
    );
    assert_eq!(
        ApiError::InvalidArgument("x".into()).kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn broken_pipes_are_recognised() {
    let pipe = ApiError::Report(ReportError::Io(std::io::Error::from(
        std::io::ErrorKind::BrokenPipe,
    )));
    assert!(pipe.is_broken_pipe());
    let vfs_pipe = ApiError::Vfs(VfsError::Io {
        spec: "<output>".into(),
        op: hikasa_vfs::VfsOp::Write,
        source: std::io::Error::from(std::io::ErrorKind::BrokenPipe),
    });
    assert!(vfs_pipe.is_broken_pipe());
    assert!(!load_failed().is_broken_pipe());
}
