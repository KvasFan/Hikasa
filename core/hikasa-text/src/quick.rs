//! Rules-based quick compare (FC-025, TC-016; §2.19; D-12, D-15, D-25): [`quick_compare_bytes`], the shared
//! core of `hikasa quick --rules` and of [`RulesComparator`], the `ContentComparator` that hikasa-folder
//! receives by injection.
//!
//! ```text
//! quick_compare_bytes(L, R, format, rules, load):
//!   0. an External conversion without a runner → Error (the error of load_document, TC-016)
//!   1. L == R (source bytes)               → BinarySame
//!   1b. External conversion of both sides (as load_document does; the verdicts below see the converted bytes)
//!   2. sniff(L) or sniff(R) is binary      → BinaryDifferent (D-12)
//!   3. both documents and their keys, on the code path of compare() (identity path included);
//!      a document limit (D-43) → Error{too_large}
//!   4. equal key sequences                 → every line pair raw-equal (EOL per rules) ? RulesSame : UnimportantOnly
//!   5. orphans always important, and the line counts differ or raw-identical lines always have equal keys
//!                                          → ImportantDifferences (no alignment: linear time)
//!   6. otherwise the alignment and row classification of compare(), and its verdict(false)
//! ```
//!
//! Consistency theorem (TC-016): for any inputs and rules, `quick_compare_bytes(...)` equals
//! `compare(...).verdict(false)` (P-TC-2), with the same regex limits and External conversion runner on both
//! paths.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::cancel::Cancelled;
use hikasa_core::content::{
    ContentComparator, ContentMethod, ContentOutcome, ContentResult, ContentSource, ReadError,
};
use hikasa_core::error::{Classify, ErrorReport};
use hikasa_core::progress::Ctl;
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_formats::{
    CompiledFormat, Conversion, ConvertError, EnabledOverrides, FormatKind, FormatRegistry,
    Pattern, ProcessRunner, RegexLimits, run_external, sniff,
};
use hikasa_vfs::{BinaryOutcome, StreamOptions, compare_sources};

use crate::classify::compile_patterns;
use crate::compare::{CompareInput, Prepared};
use crate::document::{DocOrigin, LoadOptions, document_from_bytes, external_unsupported};
use crate::error::TextError;
use crate::rules::TextRules;

/// Default per-file limit of `RulesComparator` (D-25).
pub const DEFAULT_RULES_MAX_BYTES: u64 = 128 * 1024 * 1024;

/// What the quick compare needs besides the inputs and rules: the regex limits of the session's format registry
/// (for the unimportant-text patterns) and the External conversion runner, run in a temporary directory, exactly
/// as `load_document` takes it. Additive (wave 3). `Default`: the default limits and no runner (an External
/// format then fails like `load_document` without a runner).
#[derive(Clone, Copy, Default)]
pub struct QuickOptions<'a> {
    /// Regex limits of the unimportant-text patterns (the registry's `limits()`).
    pub limits: RegexLimits,
    /// External conversions: `(temporary directory, runner)`.
    pub external: Option<(&'a Path, &'a dyn ProcessRunner)>,
}

impl fmt::Debug for QuickOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickOptions")
            .field("limits", &self.limits)
            .field("external", &self.external.map(|(dir, _)| dir))
            .finish()
    }
}

/// The shared quick-compare core (used by `RulesComparator` and by `hikasa quick --rules`). Each side is
/// `(bytes, name for format detection)`; `format` is the effective format already chosen by the caller (D-15).
///
/// [`quick_compare_bytes_with`] with [`QuickOptions::default`]: default regex limits and no conversion runner.
/// The names label the warnings and the External conversion. Errors that are not cancellation (an invalid
/// unimportant pattern, a document over the D-43 limits, an External conversion that fails or has no runner)
/// become `ContentResult::Error`; cancellation is `Err(Cancelled)`. Warnings: `lossy_decode` per side,
/// `regex_runtime_error`, and the alignment's warnings when step 6 runs.
pub fn quick_compare_bytes(
    left: (&[u8], &str),
    right: (&[u8], &str),
    format: &CompiledFormat,
    rules: &TextRules,
    load: &LoadOptions,
    ctl: &Ctl<'_>,
) -> Result<ContentOutcome, Cancelled> {
    quick_compare_bytes_with(
        left,
        right,
        format,
        rules,
        load,
        &QuickOptions::default(),
        ctl,
    )
}

/// [`quick_compare_bytes`] with the session's regex limits and External conversion runner (§2.19 steps 0 and
/// 1b). Additive (wave 3).
pub fn quick_compare_bytes_with(
    left: (&[u8], &str),
    right: (&[u8], &str),
    format: &CompiledFormat,
    rules: &TextRules,
    load: &LoadOptions,
    opts: &QuickOptions<'_>,
    ctl: &Ctl<'_>,
) -> Result<ContentOutcome, Cancelled> {
    ctl.check()?;
    // An External format without a runner fails before anything else, as load_document does.
    if matches!(format.conversion(), Conversion::External(_)) && opts.external.is_none() {
        let name = if left.1.is_empty() { right.1 } else { left.1 };
        return Ok(outcome(
            error_result(&external_unsupported(name, format)),
            Vec::new(),
        ));
    }
    // Steps 1 and 2 need no rules: identical bytes and binary content are decided before the patterns compile.
    if left.0 == right.0 {
        return Ok(outcome(ContentResult::BinarySame, Vec::new()));
    }
    if !matches!(format.conversion(), Conversion::External(_))
        && (sniff(left.0).is_binary || sniff(right.0).is_binary)
    {
        return Ok(outcome(ContentResult::BinaryDifferent, Vec::new()));
    }
    let patterns = match compile_patterns(rules, &opts.limits) {
        Ok(p) => p,
        Err(e) => return Ok(outcome(error_result(&e), Vec::new())),
    };
    quick_with_patterns(
        left,
        right,
        Arc::new(format.clone()),
        rules,
        patterns,
        load,
        opts.external,
        ctl,
    )
}

fn outcome(result: ContentResult, warnings: Vec<Warning>) -> ContentOutcome {
    ContentOutcome {
        method: ContentMethod::Rules,
        result,
        warnings,
    }
}

fn error_result(e: &TextError) -> ContentResult {
    ContentResult::Error {
        error: ErrorReport::capture(e, None),
    }
}

/// The External conversion of one side (step 1b): the converted bytes, or the error of `load_document`.
fn convert(
    bytes: &[u8],
    name: &str,
    format: &CompiledFormat,
    external: Option<(&Path, &dyn ProcessRunner)>,
    ctl: &Ctl<'_>,
) -> Result<Option<Vec<u8>>, TextError> {
    let Conversion::External(conv) = format.conversion() else {
        return Ok(None);
    };
    let Some((temp_dir, runner)) = external else {
        return Err(external_unsupported(name, format));
    };
    run_external(conv, bytes, name, temp_dir, runner, ctl.cancel)
        .map(Some)
        .map_err(|e| match e {
            ConvertError::Cancelled => TextError::Cancelled,
            other => TextError::Convert(other),
        })
}

/// [`quick_compare_bytes_with`] with the patterns already compiled.
#[allow(clippy::too_many_arguments)]
fn quick_with_patterns(
    left: (&[u8], &str),
    right: (&[u8], &str),
    format: Arc<CompiledFormat>,
    rules: &TextRules,
    patterns: Arc<[Pattern]>,
    load: &LoadOptions,
    external: Option<(&Path, &dyn ProcessRunner)>,
    ctl: &Ctl<'_>,
) -> Result<ContentOutcome, Cancelled> {
    ctl.check()?;
    let fail = |e: TextError, warnings: Vec<Warning>| {
        if e.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(outcome(error_result(&e), warnings))
        }
    };
    // 1. Identical source bytes.
    if left.0 == right.0 {
        return Ok(outcome(ContentResult::BinarySame, Vec::new()));
    }
    // 1b. The External conversion (load_document's byte stage).
    let (left_converted, right_converted) = match (
        convert(left.0, left.1, &format, external, ctl),
        convert(right.0, right.1, &format, external, ctl),
    ) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => return fail(e, Vec::new()),
    };
    let converted = left_converted.is_some();
    let lb: &[u8] = left_converted.as_deref().unwrap_or(left.0);
    let rb: &[u8] = right_converted.as_deref().unwrap_or(right.0);
    // 2. Binary content never gets a rules-based verdict (D-12).
    if sniff(lb).is_binary || sniff(rb).is_binary {
        return Ok(outcome(ContentResult::BinaryDifferent, Vec::new()));
    }
    // 3. Documents and keys.
    let doc = |bytes: &[u8], source: &[u8]| {
        document_from_bytes(
            bytes,
            &format,
            load,
            DocOrigin {
                spec: None,
                mtime: None,
                size: source.len() as u64,
            },
            converted,
            ctl.cancel,
        )
    };
    let (l, r) = match (doc(lb, left.0), doc(rb, right.0)) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => return fail(e, Vec::new()),
    };
    drop(left_converted);
    drop(right_converted);
    let mut warnings = Vec::new();
    for (doc, name) in [(&l, left.1), (&r, right.1)] {
        if doc.malformed() > 0 {
            warnings.push(Warning {
                code: WarningCode::LossyDecode,
                message: format!(
                    "{name}: {} malformed byte sequence(s) replaced with U+FFFD while decoding as {}",
                    doc.malformed(),
                    doc.encoding().encoding
                ),
                spec: None,
            });
        }
    }
    let input = CompareInput {
        left: Arc::new(l),
        right: Arc::new(r),
        format,
        rules: rules.clone(),
        bytes_identical: false,
        force_tokenize: false,
    };
    let prepared = match Prepared::new(input, patterns, None, ctl, None) {
        Ok(p) => p,
        Err(e) => return fail(e, warnings),
    };
    let regex_errors = prepared.regex_errors();
    let regex_warning = |warnings: &mut Vec<Warning>| {
        if regex_errors > 0 {
            warnings.push(Warning {
                code: WarningCode::RegexRuntimeError,
                message: format!(
                    "{regex_errors} regular-expression match attempt(s) hit the backtracking limit and counted as no match"
                ),
                spec: None,
            });
        }
    };
    // 4. Equal key sequences.
    if prepared.keys_equal(ctl.cancel)? {
        let result = if prepared.raw_equal_pairwise(ctl.cancel)? {
            ContentResult::RulesSame
        } else {
            ContentResult::UnimportantOnly
        };
        regex_warning(&mut warnings);
        return Ok(outcome(result, warnings));
    }
    // 5. The shortcut: some row differs, and every difference is important. With differing line counts some
    //    row is an orphan; with equal counts every Changed pair is important unless it is raw-identical, which
    //    context-free keys rule out (§2.19 step 5, as amended in wave 3).
    if rules.importance.orphans_always_important
        && (prepared.left.line_count() != prepared.right.line_count()
            || prepared.keys_context_free(ctl.cancel)?)
    {
        regex_warning(&mut warnings);
        return Ok(outcome(ContentResult::ImportantDifferences, warnings));
    }
    // 6. The full alignment and classification. The per-side decode warnings above already name the sides;
    //    the comparison's own copies (labelled `<memory>`) are skipped.
    match prepared.finish(None, ctl, None) {
        Ok(cmp) => {
            warnings.extend(
                cmp.warnings()
                    .iter()
                    .filter(|w| {
                        !matches!(
                            w.code,
                            WarningCode::LossyDecode | WarningCode::BinaryContent
                        )
                    })
                    .cloned(),
            );
            Ok(outcome(cmp.verdict(false), warnings))
        }
        Err(e) => fail(e, warnings),
    }
}

/// The External conversion runner of a [`RulesComparator`].
#[derive(Clone)]
struct External {
    temp_dir: PathBuf,
    runner: Arc<dyn ProcessRunner>,
}

impl fmt::Debug for External {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("External")
            .field("temp_dir", &self.temp_dir)
            .finish_non_exhaustive()
    }
}

/// Rules-based `ContentComparator` (injected into hikasa-folder by the session layer).
#[derive(Debug, Clone)]
pub struct RulesComparator {
    registry: Arc<FormatRegistry>,
    overrides: EnabledOverrides,
    forced: Option<Arc<CompiledFormat>>,
    rules: TextRules,
    patterns: Arc<[Pattern]>,
    load: LoadOptions,
    max_bytes: u64,
    stream: StreamOptions,
    external: Option<External>,
}

impl RulesComparator {
    /// Validates the rules up front (T9), with the registry's regex limits (the same limits compile the
    /// unimportant patterns; a session's text compare passes them to `compare_with_limits`).
    pub fn new(
        registry: Arc<FormatRegistry>,
        rules: TextRules,
        load: LoadOptions,
    ) -> Result<RulesComparator, TextError> {
        rules.validate(registry.limits())?;
        let patterns = compile_patterns(&rules, registry.limits())?;
        Ok(RulesComparator {
            registry,
            overrides: EnabledOverrides::default(),
            forced: None,
            rules,
            patterns,
            load,
            max_bytes: DEFAULT_RULES_MAX_BYTES,
            stream: StreamOptions::default(),
            external: None,
        })
    }

    /// Per-session enabled overrides (FF-002).
    pub fn with_overrides(self, overrides: EnabledOverrides) -> RulesComparator {
        RulesComparator { overrides, ..self }
    }

    /// Forces one format for every pair.
    pub fn with_forced_format(self, format: Arc<CompiledFormat>) -> RulesComparator {
        RulesComparator {
            forced: Some(format),
            ..self
        }
    }

    /// Per-file limit (default `DEFAULT_RULES_MAX_BYTES`); `size_limit()` returns it. Larger files are
    /// compared by bytes with warning `rules_size_limit` (D-25).
    pub fn with_max_bytes(self, max: u64) -> RulesComparator {
        RulesComparator {
            max_bytes: max,
            ..self
        }
    }

    /// The External conversion runner (the session's, run in `temp_dir`), so that External formats are
    /// converted exactly as `text compare` converts them (TC-016). Without it, a pair whose format needs an
    /// External conversion is an `Error` result (the error of `load_document` without a runner). Additive
    /// (wave 3).
    pub fn with_external(
        self,
        temp_dir: PathBuf,
        runner: Arc<dyn ProcessRunner>,
    ) -> RulesComparator {
        RulesComparator {
            external: Some(External { temp_dir, runner }),
            ..self
        }
    }

    /// Stream options of the byte comparisons (files over the size limit, D-25; non-text kinds, D-11): the
    /// session passes the options of its binary comparator (buffer size, threads; §3.3). Default
    /// `StreamOptions::default()`. Additive (wave 3).
    pub fn with_stream_options(self, stream: StreamOptions) -> RulesComparator {
        RulesComparator { stream, ..self }
    }

    /// The rules.
    pub fn rules(&self) -> &TextRules {
        &self.rules
    }

    /// The effective format of a pair (D-15): forced, else the detection of the left name when it is not
    /// empty, else of the right name, else Everything Else; a `format_mismatch` warning when both names are
    /// detected differently. The right side is detected only for that warning: when its detection fails (a
    /// user format that does not compile), the left format is still used and the warning says so.
    fn effective_format(
        &self,
        left: &str,
        right: &str,
        warnings: &mut Vec<Warning>,
    ) -> Result<Arc<CompiledFormat>, TextError> {
        if let Some(forced) = &self.forced {
            return Ok(Arc::clone(forced));
        }
        let detect = |name: &str| self.registry.detect(name, &self.overrides);
        match (left.is_empty(), right.is_empty()) {
            (false, false) => {
                let l = detect(left)?;
                match detect(right) {
                    Ok(r) if r.format.id() != l.format.id() => warnings.push(Warning {
                        code: WarningCode::FormatMismatch,
                        message: format!(
                            "{left} is detected as {}, {right} as {}; {} is used for both",
                            l.format.name(),
                            r.format.name(),
                            l.format.name()
                        ),
                        spec: None,
                    }),
                    Ok(_) => {}
                    Err(e) => warnings.push(Warning {
                        code: WarningCode::FormatMismatch,
                        message: format!(
                            "{right}: its format could not be detected ({e}); {} (the format of {left}) is used \
                             for both",
                            l.format.name()
                        ),
                        spec: None,
                    }),
                }
                Ok(l.format)
            }
            (false, true) => Ok(detect(left)?.format),
            (true, false) => Ok(detect(right)?.format),
            (true, true) => Ok(Arc::clone(self.registry.everything_else())),
        }
    }

    /// The byte comparison of D-25 (size limit) and D-11 (non-text kinds).
    fn by_bytes(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
        warnings: Vec<Warning>,
    ) -> Result<ContentOutcome, Cancelled> {
        let result = match compare_sources(left, right, &self.stream, ctl) {
            Ok(BinaryOutcome::Same) => ContentResult::BinarySame,
            Ok(BinaryOutcome::Different { .. }) => ContentResult::BinaryDifferent,
            Err(e) if e.is_cancelled() => return Err(Cancelled),
            Err(e) => ContentResult::Error {
                error: ErrorReport::capture(&e, None),
            },
        };
        Ok(outcome(result, warnings))
    }
}

impl ContentComparator for RulesComparator {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        Some(self.max_bytes)
    }

    /// §2.19: (a) a length over the limit → byte comparison with `rules_size_limit` (D-25); (b) both sides read
    /// with `read_all(max_bytes)`; (c) the effective format per D-15, a non-Text/Table kind → byte comparison
    /// (D-11); (d) `quick_compare_bytes_with` with the registry's limits and the External runner.
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        ctl.check()?;
        let size_limit_warning = |what: &str| Warning {
            code: WarningCode::RulesSizeLimit,
            message: format!(
                "{what} is larger than the rules-based comparison limit of {} bytes; compared by bytes",
                self.max_bytes
            ),
            spec: None,
        };
        // (a) Known lengths over the limit.
        for source in [left, right] {
            if source.len().is_some_and(|len| len > self.max_bytes) {
                let w = size_limit_warning(&source.display());
                return self.by_bytes(left, right, ctl, vec![w]);
            }
        }
        // (b) Read both sides.
        let read = |source: &dyn ContentSource| source.read_all(self.max_bytes, ctl.cancel);
        let (lb, rb) = match (read(left), read(right)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(ReadError::Cancelled), _) | (_, Err(ReadError::Cancelled)) => {
                return Err(Cancelled);
            }
            (Err(ReadError::TooLarge { .. }), _) => {
                let w = size_limit_warning(&left.display());
                return self.by_bytes(left, right, ctl, vec![w]);
            }
            (_, Err(ReadError::TooLarge { .. })) => {
                let w = size_limit_warning(&right.display());
                return self.by_bytes(left, right, ctl, vec![w]);
            }
            (Err(e), _) => {
                return Ok(outcome(
                    ContentResult::Error {
                        error: ErrorReport {
                            kind: e.kind(),
                            message: format!("{}: {e}", left.display()),
                            spec: Some(left.display()),
                            causes: Vec::new(),
                        },
                    },
                    Vec::new(),
                ));
            }
            (_, Err(e)) => {
                return Ok(outcome(
                    ContentResult::Error {
                        error: ErrorReport {
                            kind: e.kind(),
                            message: format!("{}: {e}", right.display()),
                            spec: Some(right.display()),
                            causes: Vec::new(),
                        },
                    },
                    Vec::new(),
                ));
            }
        };
        // (c) The effective format (D-15); non-text kinds by bytes (D-11).
        let mut warnings = Vec::new();
        let format = match self.effective_format(left.name(), right.name(), &mut warnings) {
            Ok(f) => f,
            Err(e) => return Ok(outcome(error_result(&e), warnings)),
        };
        if !matches!(format.kind(), FormatKind::Text | FormatKind::Table) {
            let result = if lb == rb {
                ContentResult::BinarySame
            } else {
                ContentResult::BinaryDifferent
            };
            return Ok(outcome(result, warnings));
        }
        // (d) The shared core.
        let external = self
            .external
            .as_ref()
            .map(|x| (x.temp_dir.as_path(), &*x.runner));
        if matches!(format.conversion(), Conversion::External(_)) && external.is_none() {
            let name = if left.name().is_empty() {
                right.display()
            } else {
                left.display()
            };
            return Ok(outcome(
                error_result(&external_unsupported(&name, &format)),
                warnings,
            ));
        }
        let mut out = quick_with_patterns(
            (&lb, left.name()),
            (&rb, right.name()),
            format,
            &self.rules,
            Arc::clone(&self.patterns),
            &self.load,
            external,
            ctl,
        )?;
        warnings.append(&mut out.warnings);
        out.warnings = warnings;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_steps() {
        let ee = CompiledFormat::everything_else();
        let rules = TextRules::default();
        let load = LoadOptions::default();
        let ctl = Ctl::detached();
        let q = |l: &[u8], r: &[u8], rules: &TextRules| {
            quick_compare_bytes((l, "l.txt"), (r, "r.txt"), &ee, rules, &load, &ctl)
                .unwrap()
                .result
        };
        assert_eq!(q(b"a\n", b"a\n", &rules), ContentResult::BinarySame);
        assert_eq!(
            q(b"a\x00", b"b\x00", &rules),
            ContentResult::BinaryDifferent
        );
        assert_eq!(q(b"a\n", b"a\r\n", &rules), ContentResult::RulesSame);
        assert_eq!(
            q(b"a\n", b"b\n", &rules),
            ContentResult::ImportantDifferences
        );
        let mut ws = TextRules::default();
        ws.importance.leading_whitespace = false;
        assert_eq!(q(b"  a\n", b"a\n", &ws), ContentResult::UnimportantOnly);
        let mut lenient = ws.clone();
        lenient.importance.orphans_always_important = false;
        assert_eq!(
            q(b"a\n\n", b"a\n", &lenient),
            ContentResult::UnimportantOnly
        );
        assert_eq!(
            q(b"a\n\n", b"a\n", &ws),
            ContentResult::ImportantDifferences
        );
        let mut bad = TextRules::default();
        bad.importance
            .unimportant
            .push(crate::rules::TextPattern::regex("("));
        assert!(matches!(q(b"a", b"b", &bad), ContentResult::Error { .. }));
    }
}
