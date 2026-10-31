//! External conversions (FF-030, FF-031; §2.10.8, D-39): [`ProcessRunner`], [`StdProcessRunner`],
//! [`ConversionVars`], [`build_argv`] and [`run_external`].
//!
//! Command lines are split into argv by Hikasa's own quoting rules and run **without a shell**:
//!
//! - tokens are separated by unquoted ASCII whitespace;
//! - `"…"` groups (anywhere inside a token); the quotes are removed, and `""` inside quotes is a literal `"`;
//! - backslash is **not** an escape, so Windows paths survive;
//! - after splitting, `%s` (source), `%t` (target) and `%o` (original) are substituted inside tokens, so
//!   substituted values are never re-split; `%%` is `%`; any other `%x` is a template error;
//! - an empty template or an empty `argv[0]` is a template error.
//!
//! The Sorted conversion (D-38) is a text-stage operation: [`sort_lines`].
//!
//! Allow-point of the architecture test (§6.1, check 2): [`StdProcessRunner`] is the one place in the core
//! crates that may read the monotonic clock (`Instant::now`, `.elapsed()`) and call `thread::sleep`, to poll
//! the child every 20 ms for cancellation and the timeout. The clock decides only *when* the call returns,
//! never what a successful conversion produces (T1, T6).

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hikasa_core::cancel::CancelToken;

use crate::error::ConvertError;
use crate::model::ExternalConversion;

/// Runs external programs (injected; tests use a fake; the session layer uses `StdProcessRunner`).
pub trait ProcessRunner: Send + Sync {
    /// Runs `argv` (`argv[0]` = program) and waits; kills the child on cancellation or timeout.
    fn run(
        &self,
        argv: &[OsString],
        timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError>;
}

/// Poll interval of [`StdProcessRunner`] (§5.3: every 20 ms while waiting for an external conversion).
const POLL: Duration = Duration::from_millis(20);

/// Windows: a console program started by the GUI, which has no console of its own, would open a console window. The
/// output is discarded either way.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Other targets: nothing to do.
#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// `std::process::Command` runner (no shell; stdin null; stdout/stderr discarded; polls `try_wait`
/// every 20 ms to honour cancellation and timeout).
#[derive(Debug, Clone, Copy, Default)]
pub struct StdProcessRunner;

impl ProcessRunner for StdProcessRunner {
    fn run(
        &self,
        argv: &[OsString],
        timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        let Some((program, args)) = argv.split_first() else {
            return Err(ConvertError::Template("empty command line".into()));
        };
        let name = program.to_string_lossy().into_owned();
        cancel.check()?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        let mut child = command.spawn().map_err(|source| ConvertError::Spawn {
            program: name.clone(),
            source,
        })?;
        // Allow-point (§6.1 check 2): the timeout is measured with the monotonic clock.
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ConvertError::Io(e));
                }
            }
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ConvertError::Cancelled);
            }
            if let Some(limit) = timeout
                && started.elapsed() >= limit
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ConvertError::Timeout {
                    program: name,
                    secs: u32::try_from(limit.as_secs()).unwrap_or(u32::MAX),
                });
            }
            // Allow-point (§6.1 check 2): polling sleep.
            std::thread::sleep(POLL);
        }
    }
}

/// Values substituted into command templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionVars {
    /// `%s`: path of the (temporary copy of the) source.
    pub source: PathBuf,
    /// `%t`: path the program must write.
    pub target: PathBuf,
    /// `%o`: original spec text (display only).
    pub original: String,
}

/// Splits a template into tokens (quotes removed), before substitution.
fn split_template(template: &str) -> Result<Vec<String>, ConvertError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_quotes = false;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => {
                in_quotes = true;
                has_token = true;
            }
            c if c.is_ascii_whitespace() && !in_quotes => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if in_quotes {
        return Err(ConvertError::Template(format!(
            "unbalanced quote in {template:?}"
        )));
    }
    if has_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Substitutes `%s`, `%t`, `%o` and `%%` inside one token.
fn substitute(token: &str, vars: &ConversionVars) -> Result<OsString, ConvertError> {
    let mut out = OsString::with_capacity(token.len());
    let mut literal = String::new();
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            literal.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => literal.push('%'),
            Some(v @ ('s' | 't' | 'o')) => {
                out.push(std::mem::take(&mut literal));
                match v {
                    's' => out.push(vars.source.as_os_str()),
                    't' => out.push(vars.target.as_os_str()),
                    _ => out.push(&vars.original),
                }
            }
            Some(other) => {
                return Err(ConvertError::Template(format!(
                    "unknown variable %{other} (expected %s, %t, %o or %%)"
                )));
            }
            None => {
                return Err(ConvertError::Template(
                    "a lone % at the end of a token (write %% for a literal %)".into(),
                ));
            }
        }
    }
    out.push(literal);
    Ok(out)
}

/// Splits `template` into argv and substitutes variables (pure; D-39, §2.10.8).
pub fn build_argv(template: &str, vars: &ConversionVars) -> Result<Vec<OsString>, ConvertError> {
    let tokens = split_template(template)?;
    if tokens.is_empty() {
        return Err(ConvertError::Template("empty command line".into()));
    }
    let argv = tokens
        .iter()
        .map(|t| substitute(t, vars))
        .collect::<Result<Vec<_>, _>>()?;
    if argv.first().is_none_or(|p| p.is_empty()) {
        return Err(ConvertError::Template("empty program name".into()));
    }
    Ok(argv)
}

/// The Sorted conversion (FF-030, D-38): lines (split at CR, LF and CRLF) sorted by Unicode code point with a
/// stable sort; every output line gets the document's dominant line ending (the most frequent one; ties: LF,
/// then CRLF, then CR), and the result ends with a line ending only if `text` did. Pure and total.
pub fn sort_lines(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut lines: Vec<&str> = Vec::new();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                lf += 1;
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if bytes.get(i + 1) == Some(&b'\n') {
                    crlf += 1;
                    i += 2;
                } else {
                    cr += 1;
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    let ends_with_eol = start == bytes.len() && !bytes.is_empty();
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    let eol = if lf >= crlf && lf >= cr {
        "\n"
    } else if crlf >= cr {
        "\r\n"
    } else {
        "\r"
    };
    // `str` ordering is byte ordering of UTF-8, which is code point order; `sort` is stable.
    lines.sort();
    let mut out = String::with_capacity(text.len());
    for (k, line) in lines.iter().enumerate() {
        out.push_str(line);
        if k + 1 < lines.len() || ends_with_eol {
            out.push_str(eol);
        }
    }
    out
}

/// Conversion directories created by this process: an input to the directory names only (never to a result).
static CONVERSIONS: AtomicU64 = AtomicU64::new(0);

/// Name attempts before giving up (every taken name is skipped, never reused).
const NAME_ATTEMPTS: u32 = 64;

/// The private directory of one conversion: `<temp_dir>/hikasa-conv-<16 hex digits>`, holding the source
/// copy (`%s`) and the target (`%t`).
///
/// It is created exclusively (`mkdir` fails on any existing entry, a symbolic link included, so nothing
/// planted in a shared temporary folder is ever followed) and, on Unix, with mode 0700, so nobody else can
/// plant anything inside. Its name is a hash of injected data (the temporary folder, the original name, the
/// input's length and first bytes) with the process id and a per-process counter: hard to guess without
/// any clock or randomness (T1); a taken name is skipped. The directory and everything in it are removed
/// on drop, on every path (success, error, timeout, cancellation, unwinding), after the runner has reaped
/// the child; the removal is retried when a descendant of the program wrote into it meanwhile.
struct PrivateDir {
    path: PathBuf,
}

impl PrivateDir {
    fn create(temp_dir: &Path, input: &[u8], original: &str) -> Result<PrivateDir, ConvertError> {
        let mut seed = DefaultHasher::new();
        std::process::id().hash(&mut seed);
        temp_dir.hash(&mut seed);
        original.hash(&mut seed);
        input.len().hash(&mut seed);
        input.get(..input.len().min(4096)).hash(&mut seed);
        let seed = seed.finish();
        // `mut` is only needed for the Unix permission bits below.
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        for _ in 0..NAME_ATTEMPTS {
            let n = CONVERSIONS.fetch_add(1, Ordering::Relaxed);
            let mut name = DefaultHasher::new();
            seed.hash(&mut name);
            n.hash(&mut name);
            let path = temp_dir.join(format!("hikasa-conv-{:016x}", name.finish()));
            match builder.create(&path) {
                Ok(()) => return Ok(PrivateDir { path }),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(ConvertError::Io(e)),
            }
        }
        Err(ConvertError::Io(std::io::Error::new(
            ErrorKind::AlreadyExists,
            "no free hikasa-conv-<n> name in the temporary folder",
        )))
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        for _ in 0..3 {
            match fs::remove_dir_all(&self.path) {
                Ok(()) => return,
                Err(e) if e.kind() == ErrorKind::NotFound => return,
                Err(_) => {}
            }
        }
    }
}

/// Runs an external load conversion: writes `input` to `hikasa-conv.src` in a private directory under
/// `temp_dir` (see `PrivateDir`), runs the program with `%s` = that file and `%t` = `hikasa-conv.out` next to
/// it, and reads `%t`. Success = exit status 0 AND non-empty target (FF-030). The directory is removed in all
/// cases.
///
/// Limitation: a timed-out or cancelled program is killed, but its own child processes are not (a process
/// group kill needs an OS API that no approved crate provides, §0.7); they lose their files with the
/// directory, which is removed after the program is reaped.
pub fn run_external(
    conv: &ExternalConversion,
    input: &[u8],
    original: &str,
    temp_dir: &Path,
    runner: &dyn ProcessRunner,
    cancel: &CancelToken,
) -> Result<Vec<u8>, ConvertError> {
    cancel.check()?;
    // Validate the template before touching the file system.
    split_template(&conv.load)?;
    let dir = PrivateDir::create(temp_dir, input, original)?;
    let source = dir.path.join("hikasa-conv.src");
    let target = dir.path.join("hikasa-conv.out");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source)?;
    file.write_all(input)?;
    file.flush()?;
    drop(file);
    let argv = build_argv(
        &conv.load,
        &ConversionVars {
            source,
            target: target.clone(),
            original: original.to_owned(),
        },
    )?;
    let program = argv
        .first()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let timeout =
        (conv.timeout_secs > 0).then(|| Duration::from_secs(u64::from(conv.timeout_secs)));
    let status = runner.run(&argv, timeout, cancel)?;
    if status != 0 {
        return Err(ConvertError::Failed { program, status });
    }
    match fs::read(&target) {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes),
        Ok(_) => Err(ConvertError::EmptyOutput { program }),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(ConvertError::EmptyOutput { program }),
        Err(e) => Err(ConvertError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> ConversionVars {
        ConversionVars {
            source: PathBuf::from("/tmp/my file.src"),
            target: PathBuf::from("/tmp/out.txt"),
            original: "C:\\a b\\x.doc".into(),
        }
    }

    fn argv(template: &str) -> Vec<String> {
        build_argv(template, &vars())
            .unwrap()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    }

    fn err(template: &str) -> String {
        build_argv(template, &vars()).unwrap_err().to_string()
    }

    #[test]
    fn sorted_conversion() {
        assert_eq!(sort_lines(""), "");
        assert_eq!(sort_lines("\n"), "\n");
        assert_eq!(sort_lines("b"), "b");
        assert_eq!(sort_lines("b\na"), "a\nb");
        assert_eq!(sort_lines("b\na\n"), "a\nb\n");
        // Code point order: uppercase before lowercase, 'é' after 'z'.
        assert_eq!(sort_lines("é\nz\nZ\n"), "Z\nz\né\n");
        // Dominant ending; ties prefer LF, then CRLF.
        assert_eq!(sort_lines("c\r\nb\r\na\n"), "a\r\nb\r\nc\r\n");
        assert_eq!(sort_lines("b\r\na\n"), "a\nb\n");
        assert_eq!(sort_lines("b\ra\r\n"), "a\r\nb\r\n");
        assert_eq!(sort_lines("b\ra\rc"), "a\rb\rc");
        assert_eq!(sort_lines("\n\nb\n"), "\n\nb\n");
    }

    #[test]
    fn build_argv_table() {
        assert_eq!(
            argv("conv %s %t"),
            ["conv", "/tmp/my file.src", "/tmp/out.txt"]
        );
        assert_eq!(argv("  conv\t-i=%s   "), ["conv", "-i=/tmp/my file.src"]);
        assert_eq!(
            argv(r#""C:\Program Files\x.exe" "%s" -o"%t""#),
            [
                r"C:\Program Files\x.exe",
                "/tmp/my file.src",
                "-o/tmp/out.txt"
            ]
        );
        assert_eq!(argv(r#"echo "say ""hi""" """#), ["echo", r#"say "hi""#, ""]);
        assert_eq!(argv("p 100%% %o"), ["p", "100%", "C:\\a b\\x.doc"]);
        assert_eq!(argv(r"p a\b c\"), ["p", r"a\b", r"c\"]);
        assert_eq!(argv("p %%s"), ["p", "%s"]);
        assert_eq!(argv("p %s%t"), ["p", "/tmp/my file.src/tmp/out.txt"]);
        assert_eq!(err(""), "command template: empty command line");
        assert_eq!(err("   "), "command template: empty command line");
        assert_eq!(err(r#""" x"#), "command template: empty program name");
        assert!(err(r#"p "x"#).starts_with("command template: unbalanced quote"));
        assert_eq!(
            err("p %x"),
            "command template: unknown variable %x (expected %s, %t, %o or %%)"
        );
        assert!(err("p 5%").contains("lone %"));
    }
}
