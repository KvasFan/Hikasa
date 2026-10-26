//! External conversions (§2.10.8, D-39, §6.2.4): P-ARGV-1 (substituted values are never split), `run_external`
//! with a fake runner (exit codes, empty output, timeout, cancellation, temporary files removed on every
//! path), and on Unix with the real `cat` and `sort` through `StdProcessRunner` (no shell).

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use common::config;
use hikasa_core::cancel::CancelToken;
use hikasa_formats::{
    ConversionVars, ConvertError, ExternalConversion, FilenameEncoding, ProcessRunner,
    StdProcessRunner, build_argv, run_external,
};
use proptest::prelude::*;

/// One template token: literal pieces and variable references.
#[derive(Debug, Clone)]
enum Piece {
    Text(String),
    Var(char),
}

fn piece() -> impl Strategy<Value = Piece> {
    prop_oneof![
        "[a-z\\\\/:. \"%-]{1,6}".prop_map(Piece::Text),
        prop::sample::select(vec!['s', 't', 'o']).prop_map(Piece::Var),
    ]
}

/// Quotes a token for the template (quotes always; `"` doubled; `%` written as `%%`).
fn quote(pieces: &[Piece]) -> String {
    let mut out = String::from("\"");
    for p in pieces {
        match p {
            Piece::Text(t) => out.push_str(&t.replace('"', "\"\"").replace('%', "%%")),
            Piece::Var(v) => {
                out.push('%');
                out.push(*v);
            }
        }
    }
    out.push('"');
    out
}

fn value() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 \"'%\\\\/:.\t-]{0,12}"
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/convert.txt"))]

    /// P-ARGV-1: substituted values are never split, re-quoted or re-substituted: every argv entry equals its
    /// template token with the values pasted in.
    #[test]
    fn p_argv_1_values_are_never_split(
        program in "[a-z]{1,8}",
        tokens in prop::collection::vec(prop::collection::vec(piece(), 1..4), 0..5),
        source in value(),
        target in value(),
        original in value(),
    ) {
        let vars = ConversionVars {
            source: PathBuf::from(&source),
            target: PathBuf::from(&target),
            original: original.clone(),
        };
        let mut template = program.clone();
        let mut expected = vec![OsString::from(&program)];
        for t in &tokens {
            template.push(' ');
            template.push_str(&quote(t));
            let mut arg = String::new();
            for p in t {
                match p {
                    Piece::Text(text) => arg.push_str(text),
                    Piece::Var('s') => arg.push_str(&source),
                    Piece::Var('t') => arg.push_str(&target),
                    Piece::Var(_) => arg.push_str(&original),
                }
            }
            expected.push(OsString::from(arg));
        }
        let argv = build_argv(&template, &vars).unwrap();
        prop_assert_eq!(argv, expected, "template {}", template);
    }
}

/// A fake runner: records argv, writes `output` to `%t` (the token after `-o`), returns `status`.
struct Fake {
    status: i32,
    output: &'static [u8],
    seen: Mutex<Vec<Vec<OsString>>>,
    sleep: Option<Duration>,
}

impl Fake {
    fn new(status: i32, output: &'static [u8]) -> Fake {
        Fake {
            status,
            output,
            seen: Mutex::new(Vec::new()),
            sleep: None,
        }
    }
}

impl ProcessRunner for Fake {
    fn run(
        &self,
        argv: &[OsString],
        timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        self.seen.lock().unwrap().push(argv.to_vec());
        // The source file exists while the program runs.
        assert!(Path::new(&argv[1]).is_file());
        if cancel.is_cancelled() {
            return Err(ConvertError::Cancelled);
        }
        if let (Some(sleep), Some(limit)) = (self.sleep, timeout)
            && sleep > limit
        {
            return Err(ConvertError::Timeout {
                program: argv[0].to_string_lossy().into_owned(),
                secs: limit.as_secs() as u32,
            });
        }
        if !self.output.is_empty() {
            std::fs::write(&argv[3], self.output).unwrap();
        }
        Ok(self.status)
    }
}

fn conv(load: &str) -> ExternalConversion {
    ExternalConversion {
        load: load.into(),
        save: None,
        filename_encoding: FilenameEncoding::Unicode,
        timeout_secs: 60,
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn run_external_with_a_fake_runner() {
    let dir = tempfile::tempdir().unwrap();
    let never = CancelToken::never();
    let ok = Fake::new(0, b"converted");
    let out = run_external(
        &conv("tool %s -o %t \"%o\""),
        b"raw",
        "C:\\in put.doc",
        dir.path(),
        &ok,
        &never,
    )
    .unwrap();
    assert_eq!(out, b"converted");
    let seen = ok.seen.lock().unwrap();
    assert_eq!(seen[0][0], "tool");
    let source = Path::new(&seen[0][1]);
    let target = Path::new(&seen[0][3]);
    assert_eq!(source.file_name().unwrap(), "hikasa-conv.src");
    assert_eq!(target.file_name().unwrap(), "hikasa-conv.out");
    let private = source.parent().unwrap();
    assert_eq!(target.parent().unwrap(), private);
    assert_eq!(private.parent().unwrap(), dir.path());
    let name = private.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("hikasa-conv-") && name.len() == "hikasa-conv-".len() + 16,
        "{name}"
    );
    assert_eq!(seen[0][4], "C:\\in put.doc");
    assert!(
        leftovers(dir.path()).is_empty(),
        "temporary files are removed"
    );

    let failed = Fake::new(3, b"partial");
    let err = run_external(
        &conv("tool %s -o %t"),
        b"x",
        "o",
        dir.path(),
        &failed,
        &never,
    )
    .unwrap_err();
    assert!(
        matches!(err, ConvertError::Failed { status: 3, ref program } if program == "tool"),
        "{err}"
    );
    assert!(leftovers(dir.path()).is_empty());

    let empty = Fake::new(0, b"");
    let err = run_external(
        &conv("tool %s -o %t"),
        b"x",
        "o",
        dir.path(),
        &empty,
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ConvertError::EmptyOutput { .. }), "{err}");
    assert!(leftovers(dir.path()).is_empty());

    let mut slow = Fake::new(0, b"late");
    slow.sleep = Some(Duration::from_secs(5));
    let mut c = conv("tool %s -o %t");
    c.timeout_secs = 1;
    let err = run_external(&c, b"x", "o", dir.path(), &slow, &never).unwrap_err();
    assert!(
        matches!(err, ConvertError::Timeout { secs: 1, .. }),
        "{err}"
    );
    assert!(leftovers(dir.path()).is_empty());

    let cancel = CancelToken::new();
    cancel.cancel();
    let err =
        run_external(&conv("tool %s -o %t"), b"x", "o", dir.path(), &ok, &cancel).unwrap_err();
    assert!(matches!(err, ConvertError::Cancelled), "{err}");
    let child = CancelToken::new();
    let cancelling = Fake::new(0, b"x");
    let token = child.child();
    child.cancel();
    let err = run_external(
        &conv("tool %s -o %t"),
        b"x",
        "o",
        dir.path(),
        &cancelling,
        &token,
    )
    .unwrap_err();
    assert!(matches!(err, ConvertError::Cancelled));
    assert!(leftovers(dir.path()).is_empty());

    let err = run_external(&conv("tool \"%s"), b"x", "o", dir.path(), &ok, &never).unwrap_err();
    assert!(matches!(err, ConvertError::Template(_)), "{err}");
    let err = run_external(&conv("tool %q"), b"x", "o", dir.path(), &ok, &never).unwrap_err();
    assert!(matches!(err, ConvertError::Template(_)), "{err}");
    assert!(leftovers(dir.path()).is_empty());
}

/// Every conversion gets its own exclusively created private directory with an unpredictable name; nothing
/// that already exists in the shared temporary folder is followed, reused or removed (wave-2 review: a
/// planted `.out` symlink was written through).
#[test]
fn private_directories_never_follow_planted_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hikasa-conv-0.src"), b"busy").unwrap();
    std::fs::write(dir.path().join("hikasa-conv-1.out"), b"busy").unwrap();
    #[cfg(unix)]
    let victim = dir.path().join("victim-file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, dir.path().join("hikasa-conv-0.out")).unwrap();
    let ok = Fake::new(0, b"y");
    for _ in 0..2 {
        run_external(
            &conv("tool %s -o %t"),
            b"x",
            "o",
            dir.path(),
            &ok,
            &CancelToken::never(),
        )
        .unwrap();
    }
    let seen = ok.seen.lock().unwrap();
    let first = Path::new(&seen[0][1]).parent().unwrap().to_path_buf();
    let second = Path::new(&seen[1][1]).parent().unwrap().to_path_buf();
    assert_ne!(first, second, "a fresh directory per conversion");
    let mut left = leftovers(dir.path());
    left.sort();
    #[cfg(unix)]
    {
        assert_eq!(
            left,
            [
                "hikasa-conv-0.out",
                "hikasa-conv-0.src",
                "hikasa-conv-1.out"
            ]
        );
        assert!(!victim.exists(), "the planted symlink was not followed");
    }
    #[cfg(not(unix))]
    assert_eq!(left, ["hikasa-conv-0.src", "hikasa-conv-1.out"]);
}

/// The private directory is created with mode 0700 (Unix): nobody else can plant files in it.
#[cfg(unix)]
#[test]
fn private_directory_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    struct Inspect(Mutex<Option<u32>>);
    impl ProcessRunner for Inspect {
        fn run(
            &self,
            argv: &[OsString],
            _timeout: Option<Duration>,
            _cancel: &CancelToken,
        ) -> Result<i32, ConvertError> {
            let private = Path::new(&argv[1]).parent().unwrap();
            let mode = std::fs::metadata(private).unwrap().permissions().mode();
            *self.0.lock().unwrap() = Some(mode & 0o777);
            std::fs::write(&argv[2], b"ok").unwrap();
            Ok(0)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let runner = Inspect(Mutex::new(None));
    run_external(
        &conv("tool %s %t"),
        b"x",
        "o",
        dir.path(),
        &runner,
        &CancelToken::never(),
    )
    .unwrap();
    assert_eq!(*runner.0.lock().unwrap(), Some(0o700));
    assert!(leftovers(dir.path()).is_empty());
}

/// A timed-out program is killed and its directory removed after it is reaped; a descendant that outlives it
/// cannot write its target afterwards (the directory is gone). Killing descendants needs a process-group kill,
/// which no approved crate provides (documented limitation, §2.10.8).
#[cfg(unix)]
#[test]
fn descendants_cannot_recreate_the_target_after_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = conv("sh -c \"(sleep 2; echo leaked > $1) & sleep 30\" sh %t");
    c.timeout_secs = 1;
    let err = run_external(
        &c,
        b"secret document",
        "x.doc",
        dir.path(),
        &StdProcessRunner,
        &CancelToken::never(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ConvertError::Timeout { secs: 1, .. }),
        "{err}"
    );
    assert!(leftovers(dir.path()).is_empty());
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        leftovers(dir.path()).is_empty(),
        "{:?}",
        leftovers(dir.path())
    );
}

#[test]
fn std_runner_errors() {
    let never = CancelToken::never();
    let err = StdProcessRunner
        .run(
            &[OsString::from("hikasa-no-such-program-xyz")],
            None,
            &never,
        )
        .unwrap_err();
    assert!(matches!(err, ConvertError::Spawn { .. }), "{err}");
    let err = StdProcessRunner.run(&[], None, &never).unwrap_err();
    assert!(matches!(err, ConvertError::Template(_)));
}

#[cfg(unix)]
#[test]
fn std_runner_with_cat_and_sort() {
    let dir = tempfile::tempdir().unwrap();
    let never = CancelToken::never();
    // `cat %s` writes to stdout, which is discarded: no target file → EmptyOutput (FF-030).
    let err = run_external(
        &conv("cat %s"),
        b"abc",
        "o",
        dir.path(),
        &StdProcessRunner,
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ConvertError::EmptyOutput { .. }), "{err}");
    // `cp` copies the source to the target; `sort -o` sorts into it. No shell is involved: the argument with a
    // space and a `;` is passed verbatim.
    let out = run_external(
        &conv("cp %s %t"),
        b"copy me",
        "o",
        dir.path(),
        &StdProcessRunner,
        &never,
    )
    .unwrap();
    assert_eq!(out, b"copy me");
    let out = run_external(
        &conv("sort -o %t %s"),
        b"pear\napple\nfig\n",
        "o",
        dir.path(),
        &StdProcessRunner,
        &never,
    )
    .unwrap();
    assert_eq!(out, b"apple\nfig\npear\n");
    let err = run_external(
        &conv("sort --no-such-flag %s"),
        b"x",
        "o",
        dir.path(),
        &StdProcessRunner,
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ConvertError::Failed { .. }), "{err}");
    let err = run_external(
        &conv("cat \"%s; echo injected\""),
        b"x",
        "o",
        dir.path(),
        &StdProcessRunner,
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ConvertError::Failed { .. }), "{err}");
    assert!(leftovers(dir.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn std_runner_timeout_and_cancellation_kill_the_child() {
    let started = std::time::Instant::now();
    let err = StdProcessRunner
        .run(
            &[OsString::from("sleep"), OsString::from("10")],
            Some(Duration::from_millis(100)),
            &CancelToken::never(),
        )
        .unwrap_err();
    assert!(matches!(err, ConvertError::Timeout { .. }), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        trigger.cancel();
    });
    let started = std::time::Instant::now();
    let err = StdProcessRunner
        .run(
            &[OsString::from("sleep"), OsString::from("10")],
            None,
            &cancel,
        )
        .unwrap_err();
    t.join().unwrap();
    assert!(matches!(err, ConvertError::Cancelled), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let status = StdProcessRunner
        .run(&[OsString::from("false")], None, &CancelToken::never())
        .unwrap();
    assert_eq!(status, 1);
}
