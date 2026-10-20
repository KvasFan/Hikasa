//! Shared vocabulary of Hikasa: sides, specs, relative paths, the case and normalisation rules, the file-mask
//! engine and name filters, the injected content-comparison seam, cancellation, progress, job ids, timestamps,
//! exit codes, errors, warnings and schema versions.
//!
//! This crate depends on no other workspace crate (ARCH-010) and does no I/O except through the
//! [`ContentSource`] trait objects that callers pass in. Every function is a deterministic function of its
//! arguments: no environment variable, current directory, clock or locale is ever read here (tenet T1). The
//! only clock access is the delivery throttle of [`ThrottledSink`], which affects *when* progress is
//! forwarded, never a result.
//!
//! # Modules
//!
//! | Module | Content |
//! |---|---|
//! | [`side`] | [`Side`], [`Pair`] |
//! | [`spec`] | [`Spec`] parsing and canonical form (ARCH-020, VFS-010; D-05, D-06) |
//! | [`path`] | [`RelPath`], a normalised `/`-separated path relative to a base folder |
//! | [`case`] | the only case and Unicode normalisation rules used anywhere in Hikasa |
//! | [`mask`] | the file mask engine (FC-050, FC-051; D-07, D-08) |
//! | [`filter`] | [`NameFilter`] (FC-050, FC-051, FC-054; D-09) |
//! | [`content`] | the content-comparison seam (FC-025, FC-029) |
//! | [`cancel`] | [`CancelToken`] (ARCH-003) |
//! | [`progress`] | [`Progress`], sinks and [`Ctl`] |
//! | [`job`] | [`JobId`], [`JobKind`], [`JobIdAllocator`] |
//! | [`time`] | [`FileTime`] |
//! | [`exit`] | [`ExitCode`] and its mapping rules (CLI-004) |
//! | [`error`] | [`ErrorKind`], [`Classify`], [`ErrorReport`], [`InvariantViolation`] |
//! | [`warning`] | [`Warning`], [`WarningCode`] |
//! | [`schema`] | the `schema_version` check of every TOML loader (SES-052) |
//!
//! The most used items are re-exported at the crate root and in [`prelude`].
//!
//! Specification: `docs/design/m1-contracts.md` §1.1 and the algorithms of §2.1–§2.3 and §2.21.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod cancel;
pub mod case;
pub mod content;
pub mod error;
pub mod exit;
pub mod filter;
pub mod job;
pub mod mask;
pub mod path;
pub mod progress;
pub mod schema;
pub mod side;
pub mod spec;
pub mod time;
pub mod warning;

pub use cancel::{CancelToken, Cancelled};
pub use content::{
    ContentComparator, ContentKind, ContentMethod, ContentOutcome, ContentResult, ContentSource,
    ReadError,
};
pub use error::{Classify, ErrorKind, ErrorReport, InvariantViolation, io_error_kind};
pub use exit::{CompareVerdict, ExitCode, UnknownExitCode};
pub use filter::{FolderVerdict, NameFilter, NameFilterLists};
pub use job::{JobId, JobIdAllocator, JobKind};
pub use mask::{CaseSensitivity, Mask, MaskError, MaskList, MaskOptions, MaskTarget, Polarity};
pub use path::{RelPath, RelPathError};
pub use progress::{Ctl, NullProgress, Phase, Progress, ProgressSink, ThrottledSink, Unit};
pub use schema::{SchemaCompat, SchemaError, check_schema_version};
pub use side::{Pair, Side};
pub use spec::{
    HttpUrl, LocalPath, PathStyle, RemoteScheme, RemoteUrl, Secret, Spec, SpecContext, SpecError,
    normalize_local,
};
pub use time::{FileTime, TimeParseError};
pub use warning::{Warning, WarningCode};

/// One-line import of the vocabulary used by almost every Hikasa crate: `use hikasa_core::prelude::*;`.
pub mod prelude {
    pub use crate::cancel::{CancelToken, Cancelled};
    pub use crate::content::{
        ContentComparator, ContentKind, ContentMethod, ContentOutcome, ContentResult,
        ContentSource, ReadError,
    };
    pub use crate::error::{Classify, ErrorKind, ErrorReport, InvariantViolation};
    pub use crate::exit::{CompareVerdict, ExitCode};
    pub use crate::filter::{FolderVerdict, NameFilter, NameFilterLists};
    pub use crate::job::{JobId, JobKind};
    pub use crate::mask::{CaseSensitivity, Mask, MaskList, MaskOptions, MaskTarget, Polarity};
    pub use crate::path::RelPath;
    pub use crate::progress::{
        Ctl, NullProgress, Phase, Progress, ProgressSink, ThrottledSink, Unit,
    };
    pub use crate::side::{Pair, Side};
    pub use crate::spec::{LocalPath, PathStyle, Spec, SpecContext};
    pub use crate::time::FileTime;
    pub use crate::warning::{Warning, WarningCode};
}
