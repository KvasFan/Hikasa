//! FileTime edge cases, CancelToken semantics, progress throttling, Ctl laziness, the ContentSource and
//! ContentComparator seams, ErrorReport capture and Side display (§6.2.1 table-driven unit tests).

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hikasa_core::prelude::*;
use hikasa_core::time::TimeParseError;
use pretty_assertions::assert_eq;

// ---- FileTime -----------------------------------------------------------------------------------------

#[test]
fn file_time_pre_epoch() {
    // `SystemTime` offsets in these tests are multiples of 100 ns, the precision of Windows' `SystemTime`
    // (`UNIX_EPOCH - 1 ns` *is* `UNIX_EPOCH` there); nanosecond-exact values go through `from_unix`.
    let t = FileTime::from_system_time(UNIX_EPOCH - Duration::new(0, 100));
    assert_eq!((t.unix_seconds(), t.subsec_nanos()), (-1, 999_999_900));
    assert_eq!(t.to_rfc3339(), "1969-12-31T23:59:59.9999999Z");
    assert_eq!(FileTime::parse(&t.to_rfc3339()).unwrap(), t);
    let t = FileTime::from_unix(-1, 999_999_999).unwrap();
    assert_eq!(t.to_rfc3339(), "1969-12-31T23:59:59.999999999Z");
    assert_eq!(FileTime::parse(&t.to_rfc3339()).unwrap(), t);
    let t = FileTime::from_unix(-86_400, 0).unwrap();
    assert_eq!(t.to_string(), "1969-12-31T00:00:00Z");
}

#[test]
fn file_time_nanosecond_text() {
    let cases = [
        (0, 0, "1970-01-01T00:00:00Z"),
        (0, 500_000_000, "1970-01-01T00:00:00.5Z"),
        (0, 250_000_000, "1970-01-01T00:00:00.25Z"),
        (0, 1, "1970-01-01T00:00:00.000000001Z"),
        (0, 120_000, "1970-01-01T00:00:00.00012Z"),
        (1_792_415_520, 0, "2026-10-19T13:12:00Z"),
    ];
    for (secs, nanos, text) in cases {
        let t = FileTime::from_unix(secs, nanos).unwrap();
        assert_eq!(t.to_rfc3339(), text);
        assert_eq!(FileTime::parse(text).unwrap(), t);
        assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{text}\""));
    }
}

#[test]
fn file_time_offsets_are_normalised_to_utc() {
    let utc = FileTime::parse("2026-10-19T13:12:00Z").unwrap();
    assert_eq!(FileTime::parse("2026-10-19T15:12:00+02:00").unwrap(), utc);
    assert_eq!(FileTime::parse("2026-10-19T08:42:00-04:30").unwrap(), utc);
}

#[test]
fn file_time_out_of_range_fallback() {
    for (secs, nanos) in [
        (i64::MIN, 0),
        (i64::MAX, 999_999_999),
        (-400_000_000_000, 7),
        (300_000_000_000, 1),
    ] {
        let t = FileTime::from_unix(secs, nanos).unwrap();
        let text = t.to_rfc3339();
        assert_eq!(text, format!("@{secs}.{nanos:09}"));
        assert_eq!(FileTime::parse(&text).unwrap(), t);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<FileTime>(&json).unwrap(), t);
    }
    let max = jiff::Timestamp::MAX;
    let edge = FileTime::from_unix(
        max.as_second(),
        u32::try_from(max.subsec_nanosecond()).unwrap(),
    )
    .unwrap();
    assert_eq!(edge.to_rfc3339(), max.to_string());
    // RFC 3339 still writes the last day of year 9999, which is beyond jiff's `Timestamp::MAX`.
    let past_jiff = FileTime::from_unix(edge.unix_seconds() + 1, 0).unwrap();
    assert_eq!(past_jiff.to_rfc3339(), "9999-12-30T22:00:01Z");
    let last = FileTime::parse("9999-12-31T23:59:59.999999999Z").unwrap();
    let after = FileTime::from_unix(last.unix_seconds() + 1, 0).unwrap();
    assert_eq!(after.to_rfc3339(), "@253402300800.000000000");
    let before = FileTime::from_unix(-62_167_219_201, 999_999_999).unwrap();
    assert_eq!(before.to_rfc3339(), "@-62167219201.999999999");
}

#[test]
fn file_time_parse_errors() {
    let err = FileTime::parse("not a time").unwrap_err();
    assert_eq!(
        err,
        TimeParseError {
            input: "not a time".into(),
            reason: "not RFC 3339"
        }
    );
    assert_eq!(
        err.to_string(),
        "invalid timestamp \"not a time\": not RFC 3339"
    );
    assert!(
        FileTime::parse("@1.1234567890").is_err(),
        "exactly nine fractional digits"
    );
    assert!(
        FileTime::from_unix(0, 1_000_000_000).is_none(),
        "INV-TIME-1"
    );
    assert!(serde_json::from_str::<FileTime>("\"2026-13-01T00:00:00Z\"").is_err());
}

#[test]
fn file_time_from_system_time_round_trips() {
    for offset in [
        Duration::ZERO,
        Duration::new(1, 100),
        Duration::new(1_700_000_000, 123_456_700),
    ] {
        let t = FileTime::from_system_time(UNIX_EPOCH + offset);
        assert_eq!(t.unix_seconds(), i64::try_from(offset.as_secs()).unwrap());
        assert_eq!(t.subsec_nanos(), offset.subsec_nanos());
    }
    let now_ish = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let t = FileTime::from_system_time(now_ish);
    assert_eq!(
        t.nanos_until(FileTime::from_unix(1_800_000_001, 5).unwrap()),
        1_000_000_005
    );
}

// ---- CancelToken ----------------------------------------------------------------------------------------

#[test]
fn cancel_parent_child_semantics() {
    let root = CancelToken::new();
    let a = root.child();
    let b = root.child();
    let a1 = a.child();
    a.cancel();
    assert!(a.is_cancelled() && a1.is_cancelled(), "INV-CANCEL-1");
    assert!(!root.is_cancelled() && !b.is_cancelled(), "INV-CANCEL-2");
    root.cancel();
    assert!(b.is_cancelled());
    assert_eq!(b.check(), Err(Cancelled));
    assert_eq!(Cancelled.to_string(), "operation cancelled");
}

#[test]
fn cancel_is_visible_across_threads() {
    let token = CancelToken::new();
    let child = token.child();
    let seen = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            while !child.is_cancelled() {
                std::thread::yield_now();
            }
            true
        });
        token.cancel();
        handle.join().unwrap()
    });
    assert!(seen);
}

// ---- Progress -------------------------------------------------------------------------------------------

fn report(phase: Phase, done: u64, total: Option<u64>) -> Progress {
    Progress {
        phase,
        done,
        total,
        unit: Unit::Bytes,
        current: Some(format!("item {done}")),
    }
}

#[test]
fn throttled_sink_forwards_phase_changes_and_completion() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = Arc::clone(&seen);
    let sink = ThrottledSink::new(
        move |p: Progress| sink_seen.lock().unwrap().push((p.phase, p.done)),
        Duration::from_secs(3600),
    );
    sink.report(report(Phase::Reading, 0, Some(3)));
    sink.report(report(Phase::Reading, 1, Some(3)));
    sink.report(report(Phase::Reading, 2, Some(3)));
    sink.report(report(Phase::Reading, 3, Some(3)));
    sink.report(report(Phase::Decoding, 0, None));
    // Phases are throttled independently: going back to `reading` within its interval is not a new start.
    sink.report(report(Phase::Reading, 0, Some(9)));
    sink.report(report(Phase::Reading, 9, Some(9)));
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (Phase::Reading, 0),
            (Phase::Reading, 3),
            (Phase::Decoding, 0),
            (Phase::Reading, 9)
        ]
    );
}

#[test]
fn throttled_sink_bounds_interleaved_phases() {
    // Folder compare: scan tasks and content tasks report concurrently through one job sink (§5.1, §5.4).
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let sink = Arc::new(ThrottledSink::new(
        move |_: Progress| {
            counter.fetch_add(1, Ordering::Relaxed);
        },
        Duration::from_secs(3600),
    ));
    let built = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for phase in [Phase::Scanning, Phase::ComparingContent] {
            let sink = Arc::clone(&sink);
            let built = &built;
            scope.spawn(move || {
                let cancel = CancelToken::never();
                let ctl = Ctl {
                    cancel: &cancel,
                    progress: sink.as_ref(),
                };
                for done in 0..10_000 {
                    ctl.progress(phase, || {
                        built.fetch_add(1, Ordering::Relaxed);
                        report(phase, done, None)
                    });
                }
            });
        }
    });
    assert_eq!(seen.load(Ordering::Relaxed), 2, "one report per phase");
    assert_eq!(
        built.load(Ordering::Relaxed),
        2,
        "lazy reports are built only when due"
    );
}

#[test]
fn throttled_sink_reopens_after_the_interval() {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let sink = ThrottledSink::new(
        move |_: Progress| {
            counter.fetch_add(1, Ordering::Relaxed);
        },
        Duration::from_millis(5),
    );
    sink.report(report(Phase::Hashing, 0, None));
    assert!(!sink.due(Phase::Hashing));
    std::thread::sleep(Duration::from_millis(20));
    assert!(sink.due(Phase::Hashing));
    sink.report(report(Phase::Hashing, 1, None));
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn shared_sinks_can_be_throttled() {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let shared: Arc<dyn ProgressSink> = Arc::new(move |_: Progress| {
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let throttled = ThrottledSink::new(shared, Duration::ZERO);
    throttled.report(report(Phase::Scanning, 0, None));
    assert_eq!(count.load(Ordering::Relaxed), 1);
}

#[test]
fn ctl_progress_does_not_build_reports_when_not_due() {
    let built = AtomicUsize::new(0);
    let delivered = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&delivered);
    let sink = ThrottledSink::new(
        move |_: Progress| {
            counter.fetch_add(1, Ordering::Relaxed);
        },
        Duration::from_secs(3600),
    );
    let cancel = CancelToken::never();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &sink,
    };
    for done in 0..100 {
        ctl.progress(Phase::Tokenizing, || {
            built.fetch_add(1, Ordering::Relaxed);
            report(Phase::Tokenizing, done, Some(100))
        });
    }
    assert_eq!(
        built.load(Ordering::Relaxed),
        1,
        "only the first report was due"
    );
    ctl.progress_now(report(Phase::Tokenizing, 100, Some(100)));
    assert_eq!(
        delivered.load(Ordering::Relaxed),
        2,
        "completion always passes"
    );
    let detached = Ctl::detached();
    detached.progress(Phase::Tokenizing, || panic!("NullProgress is never due"));
    assert_eq!(detached.check(), Ok(()));
}

// ---- Content seam -----------------------------------------------------------------------------------------

/// An in-memory content source (test double).
struct Bytes {
    name: &'static str,
    data: Vec<u8>,
}

impl ContentSource for Bytes {
    fn name(&self) -> &str {
        self.name
    }
    fn display(&self) -> String {
        format!("mem:{}", self.name)
    }
    fn len(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
    fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(self.data.as_slice()))
    }
    fn read_all(&self, limit: u64, cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        cancel.check()?;
        if self.data.len() as u64 > limit {
            return Err(ReadError::TooLarge { limit });
        }
        let mut out = Vec::with_capacity(self.data.len());
        self.open()?.read_to_end(&mut out)?;
        Ok(out)
    }
}

/// A byte-equality comparator (test double).
struct Equal;

impl ContentComparator for Equal {
    fn method(&self) -> ContentMethod {
        ContentMethod::Binary
    }
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        ctl.check()?;
        let read = |s: &dyn ContentSource| s.read_all(u64::MAX, ctl.cancel);
        let result = match (read(left), read(right)) {
            (Ok(a), Ok(b)) => {
                if a == b {
                    ContentResult::BinarySame
                } else {
                    ContentResult::BinaryDifferent
                }
            }
            (Err(ReadError::Cancelled), _) | (_, Err(ReadError::Cancelled)) => {
                return Err(Cancelled);
            }
            (Err(e), _) | (_, Err(e)) => ContentResult::Error {
                error: ErrorReport::capture(&e, None),
            },
        };
        Ok(ContentOutcome {
            method: self.method(),
            result,
            warnings: Vec::new(),
        })
    }
}

#[test]
fn content_seam_works_with_trait_objects() {
    let a = Bytes {
        name: "a.txt",
        data: b"same".to_vec(),
    };
    let b = Bytes {
        name: "b.txt",
        data: b"same".to_vec(),
    };
    let c = Bytes {
        name: "c.txt",
        data: b"diff".to_vec(),
    };
    let comparator: Arc<dyn ContentComparator> = Arc::new(Equal);
    assert_eq!(comparator.size_limit(), None);
    assert_eq!(a.known_crc32(), None);
    assert_eq!(a.local_path(), None);
    let ctl = Ctl::detached();
    assert_eq!(
        comparator.compare(&a, &b, &ctl).unwrap().result,
        ContentResult::BinarySame
    );
    assert_eq!(
        comparator.compare(&a, &c, &ctl).unwrap().result,
        ContentResult::BinaryDifferent
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    let cancelled = Ctl {
        cancel: &cancel,
        progress: &NullProgress,
    };
    assert_eq!(comparator.compare(&a, &b, &cancelled), Err(Cancelled));
    assert!(matches!(
        a.read_all(2, &CancelToken::never()),
        Err(ReadError::TooLarge { limit: 2 })
    ));
    assert_eq!(a.display(), "mem:a.txt");
}

#[test]
fn error_reports_capture_kind_message_spec_and_causes() {
    let spec = Spec::parse(
        "ftp://u:pw@h/x",
        &SpecContext::absolute_only(PathStyle::Unix),
    )
    .unwrap();
    let err = ReadError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    let report = ErrorReport::capture(&err, Some(&spec));
    assert_eq!(report.kind, ErrorKind::PermissionDenied);
    assert_eq!(report.message, "denied");
    assert_eq!(report.spec.as_deref(), Some("ftp://u@h/x"));
    let too_large = ErrorReport::capture(&ReadError::TooLarge { limit: 10 }, None);
    assert_eq!(too_large.kind, ErrorKind::TooLarge);
    assert_eq!(too_large.message, "content larger than 10 bytes");
    assert!(too_large.causes.is_empty());
}

#[test]
fn side_display_and_pairs() {
    assert_eq!(format!("{} input", Side::Left), "left input");
    assert_eq!(Side::Right.to_string(), "right");
    assert_eq!(serde_json::to_string(&Side::Center).unwrap(), "\"center\"");
    let pair = Pair::new("l", "r");
    assert_eq!(pair.swapped(), Pair::new("r", "l"));
    assert_eq!(Side::INPUTS.map(|s| *pair.get(s).unwrap()), ["l", "r"]);
}
