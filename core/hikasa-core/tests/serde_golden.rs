//! Golden JSON shapes of every serialisable hikasa-core type (T5, §4.1). The fixtures live under
//! `tests/fixtures/serde/`; run with `HIKASA_BLESS=1` to rewrite them after an intended change.

mod common;

use hikasa_core::prelude::*;
use hikasa_core::spec::RemoteScheme;
use hikasa_core::time::FileTime;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Pretty JSON with 2-space indentation and a final newline (§4.1 rule 1).
fn to_json<T: Serialize>(value: &T) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text.into_bytes()
}

fn check<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(case: &str, value: &T) {
    let json = to_json(value);
    common::assert_golden(&common::fixture("serde", case), &json);
    let back: T = serde_json::from_slice(&json).unwrap();
    assert_eq!(&back, value, "{case} round trip");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ContentDoc {
    methods: Vec<ContentMethod>,
    kinds: Vec<ContentKind>,
    results: Vec<ContentResult>,
    outcome: ContentOutcome,
}

#[test]
fn content_shapes() {
    let error = ErrorReport {
        kind: ErrorKind::NotFound,
        message: "left input /w/missing.c: /w/missing.c: not found".into(),
        spec: Some("/w/missing.c".into()),
        causes: vec!["/w/missing.c: not found".into()],
    };
    let doc = ContentDoc {
        methods: vec![
            ContentMethod::None,
            ContentMethod::Crc,
            ContentMethod::Binary,
            ContentMethod::Rules,
        ],
        kinds: hikasa_core::content::ContentKind::ALL.to_vec(),
        results: vec![
            ContentResult::BinarySame,
            ContentResult::RulesSame,
            ContentResult::UnimportantOnly,
            ContentResult::ImportantDifferences,
            ContentResult::BinaryDifferent,
            ContentResult::Error { error },
        ],
        outcome: ContentOutcome {
            method: ContentMethod::Rules,
            result: ContentResult::BinaryDifferent,
            warnings: vec![Warning {
                code: WarningCode::RulesSizeLimit,
                message: "larger than 128 MiB: compared by bytes".into(),
                spec: Some("/w/big.log".into()),
            }],
        },
    };
    check("content.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ErrorDoc {
    kinds: Vec<ErrorKind>,
    report: ErrorReport,
    warning_codes: Vec<WarningCode>,
    warnings: Vec<Warning>,
}

#[test]
fn error_and_warning_shapes() {
    let doc = ErrorDoc {
        kinds: vec![
            ErrorKind::NotFound,
            ErrorKind::PermissionDenied,
            ErrorKind::AlreadyExists,
            ErrorKind::NotADirectory,
            ErrorKind::IsADirectory,
            ErrorKind::InvalidInput,
            ErrorKind::InvalidData,
            ErrorKind::Unsupported,
            ErrorKind::Cancelled,
            ErrorKind::Io,
            ErrorKind::TooLarge,
            ErrorKind::Protocol,
            ErrorKind::Internal,
            ErrorKind::NotEmpty,
        ],
        report: ErrorReport {
            kind: ErrorKind::Internal,
            message: "panic: boom".into(),
            spec: None,
            causes: vec![],
        },
        warning_codes: vec![
            WarningCode::BinaryContent,
            WarningCode::LossyDecode,
            WarningCode::RegexRuntimeError,
            WarningCode::FellBackToBinary,
            WarningCode::RulesSizeLimit,
            WarningCode::ClosenessIgnored,
            WarningCode::WorkBudgetExhausted,
            WarningCode::ClosenessBudgetExhausted,
            WarningCode::FormatMismatch,
            WarningCode::CapabilityMissing,
            WarningCode::UnknownConfigKey,
            WarningCode::UserFormatSkipped,
            WarningCode::PatchLossyEol,
            WarningCode::SymlinkLoop,
            WarningCode::EntryError,
            WarningCode::SwitchIgnored,
            WarningCode::AlignmentDegraded,
            WarningCode::PatchEncoding,
            WarningCode::InvalidConfigValue,
            WarningCode::SpecNotSaved,
            WarningCode::SessionMissing,
            WarningCode::SessionLocked,
        ],
        warnings: vec![
            Warning {
                code: WarningCode::FormatMismatch,
                message: "left is C, right is Python".into(),
                spec: None,
            },
            Warning {
                code: WarningCode::SymlinkLoop,
                message: "symbolic link loop".into(),
                spec: Some("/w/loop".into()),
            },
        ],
    };
    check("errors.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ExitDoc {
    codes: Vec<ExitCode>,
    verdicts: Vec<CompareVerdict>,
}

#[test]
fn exit_shapes() {
    let doc = ExitDoc {
        codes: ExitCode::ALL.to_vec(),
        verdicts: vec![
            CompareVerdict::NoDifferences,
            CompareVerdict::UnimportantOnly,
            CompareVerdict::Differences,
            CompareVerdict::Incomplete,
        ],
    };
    check("exit.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct TimeDoc {
    times: Vec<FileTime>,
    optional: Option<FileTime>,
}

#[test]
fn time_shapes() {
    let t = |secs, nanos| FileTime::from_unix(secs, nanos).unwrap();
    let doc = TimeDoc {
        times: vec![
            t(0, 0),
            t(1_792_415_520, 0),
            t(1_792_415_520, 500_000_000),
            t(1_792_415_520, 250_000_000),
            t(1_792_415_520, 1),
            t(-1, 500_000_000),
            t(-400_000_000_000, 5),
            // The ends of the RFC 3339 range (years 0000 and 9999) and the first instants beyond it.
            t(-62_167_219_200, 0),
            t(253_402_300_799, 999_999_999),
            t(-62_167_219_201, 999_999_999),
            t(253_402_300_800, 0),
        ],
        optional: None,
    };
    check("time.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ProgressDoc {
    phases: Vec<Phase>,
    units: Vec<Unit>,
    reports: Vec<Progress>,
    job: JobId,
    job_kinds: Vec<JobKind>,
    sides: Vec<Side>,
    pair: Pair<u64>,
}

#[test]
fn progress_and_job_shapes() {
    let doc = ProgressDoc {
        phases: vec![
            Phase::Reading,
            Phase::Decoding,
            Phase::Tokenizing,
            Phase::Aligning,
            Phase::Classifying,
            Phase::Scanning,
            Phase::ComparingContent,
            Phase::Hashing,
            Phase::Reporting,
            Phase::Writing,
        ],
        units: vec![Unit::Items, Unit::Bytes, Unit::Lines, Unit::Folders],
        reports: vec![
            Progress {
                phase: Phase::Scanning,
                done: 12,
                total: None,
                unit: Unit::Folders,
                current: Some("src/a".into()),
            },
            Progress {
                phase: Phase::Reading,
                done: 4096,
                total: Some(8192),
                unit: Unit::Bytes,
                current: None,
            },
        ],
        job: JobId(42),
        job_kinds: vec![
            JobKind::QuickCompare,
            JobKind::TextCompare,
            JobKind::FolderCompare,
            JobKind::Report,
            JobKind::FormatTokens,
            JobKind::VfsList,
            JobKind::VfsRead,
            JobKind::VfsWrite,
        ],
        sides: vec![Side::Left, Side::Right, Side::Center, Side::Output],
        pair: Pair::new(1, 2),
    };
    check("progress.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct FilterDoc {
    lists: NameFilterLists,
    options: Vec<MaskOptions>,
    targets: Vec<MaskTarget>,
    polarities: Vec<Polarity>,
    verdicts: Vec<FolderVerdict>,
    paths: Vec<RelPath>,
}

#[test]
fn filter_shapes() {
    let doc = FilterDoc {
        lists: NameFilterLists {
            include_files: vec!["*.c".into(), "*.h".into()],
            exclude_files: vec!["*.bak".into()],
            include_folders: vec!["src\\".into()],
            exclude_folders: vec!["obj\\".into()],
        },
        options: vec![
            MaskOptions {
                case: CaseSensitivity::Sensitive,
            },
            MaskOptions {
                case: CaseSensitivity::Insensitive,
            },
        ],
        targets: vec![MaskTarget::Files, MaskTarget::Folders],
        polarities: vec![Polarity::Include, Polarity::Exclude],
        verdicts: vec![
            FolderVerdict::Base,
            FolderVerdict::Included,
            FolderVerdict::PassThrough,
            FolderVerdict::Excluded,
        ],
        paths: vec![RelPath::root(), RelPath::parse("src/core/a.c").unwrap()],
    };
    check("filter.json", &doc);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SpecDoc {
    styles: Vec<PathStyle>,
    schemes: Vec<RemoteScheme>,
    specs: Vec<Spec>,
}

#[test]
fn portable_spec_shapes() {
    let ctx = SpecContext::absolute_only(PathStyle::HOST);
    let specs = [
        "-",
        "clipboard://",
        "profile:My Prof?dir/f.txt",
        "SFTP://u:secret@H:2222/p",
        "https://Example.com/a?b",
        "HTTP://u:secret@Example.com:8080/a",
    ]
    .iter()
    .map(|s| Spec::parse(s, &ctx).unwrap().without_secrets())
    .collect();
    let doc = SpecDoc {
        styles: vec![PathStyle::Unix, PathStyle::Windows],
        schemes: RemoteScheme::ALL.to_vec(),
        specs,
    };
    check("spec.json", &doc);
}

#[cfg(unix)]
#[test]
fn unix_local_spec_shapes() {
    let ctx = SpecContext {
        style: PathStyle::Unix,
        cwd: Some(std::path::PathBuf::from("/w")),
        home: None,
    };
    let specs: Vec<Spec> = ["/home/u//x/./y/", "a b/ü.txt", "/a/../b", "/"]
        .iter()
        .map(|s| Spec::parse(s, &ctx).unwrap())
        .collect();
    check("spec_unix.json", &specs);
}

#[test]
fn unknown_enum_names_are_rejected() {
    assert!(serde_json::from_str::<ContentKind>("\"similar\"").is_err());
    assert!(serde_json::from_str::<ExitCode>("15").is_err());
    assert!(serde_json::from_str::<RelPath>("\"/abs\"").is_err());
    assert!(serde_json::from_str::<FileTime>("\"@1\"").is_err());
}
