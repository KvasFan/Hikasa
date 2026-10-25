//! `quick`, `text compare` and `folder compare` (§3.1–§3.3).

use std::ffi::OsStr;
use std::sync::Arc;

use hikasa_core::{
    Cancelled, ContentComparator, ContentMethod, ContentOutcome, ContentResult, ContentSource,
    ExitCode, Pair, ReadError, Side, Spec, Warning, WarningCode,
};
use hikasa_folder::{
    ContentEngines, Criteria, FolderEnv, FolderModel, FolderRules, ScanObserver, compare_into,
};
use hikasa_formats::{
    CompiledFormat, DetectReason, Detection, EnabledOverrides, FormatError, FormatKind,
    TextEncoding,
};
use hikasa_text::{
    CompareInput, CompareObserver, DEFAULT_RULES_MAX_BYTES, LoadOptions, MAX_DOCUMENT_BYTES,
    QuickOptions, RulesComparator, TextDocument, TextError, TextRules, TextRulesPatch,
    compare_with_limits, load_document, quick_compare_bytes_with,
};
use hikasa_vfs::{
    BinaryComparator, BinaryOutcome, CrcComparator, Entry, EntryKind, FollowLinks, Vfs, VfsError,
    VfsOp, VfsSource, compare_sources,
};

use super::requests::{FolderCompareRequest, QuickMethod, QuickRequest, TextCompareRequest};
use super::responses::{
    FolderCompareResponse, FormatChoice, PreparedFolderCompare, QuickResponse, TextCompareResponse,
};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// Raw buffers at least this large (both sides together) are freed on a short-lived thread, so that a large
/// comparison does not wait for the allocator after `bytes_identical` (§3.2 step 3, wave-3 decision 3).
const DROP_OFF_THREAD_BYTES: usize = 32 << 20;

/// `bytes_identical` compares in steps of this size, checking cancellation between them.
const COMPARE_STEP: usize = 1 << 20;

/// The effective rules of a folder comparison ([`Service::folder_rules`]).
pub(crate) struct EffectiveFolderRules {
    pub(crate) rules: FolderRules,
    pub(crate) text_rules: Option<TextRules>,
    pub(crate) ignore_unimportant: bool,
}

/// The effective format of a file comparison with its per-side detections and warnings (D-15).
pub(crate) struct ChosenFormat {
    pub(crate) format: Arc<CompiledFormat>,
    pub(crate) choice: FormatChoice,
    pub(crate) detection: Pair<Option<DetectReason>>,
    pub(crate) warnings: Vec<Warning>,
}

impl Service {
    /// Quick compare (§3.1).
    ///
    /// Size: sizes equal ? `BinarySame` : `BinaryDifferent` (no I/O beyond `stat`). CRC and binary: the
    /// hikasa-vfs comparators with [`Service::stream_options`]. Rules: the rules (defaults ← `rules_file` ←
    /// `rules_patch`), the effective format of D-15 (`--format`, else the left name, else the right name; `-`
    /// uses the stdin name hint), then exactly the steps of `RulesComparator` (a non-Text/Table kind compares
    /// bytes, D-11; else `quick_compare_bytes_with` with the registry's regex limits and the conversion runner),
    /// with the document limit of D-43 instead of the folder comparator's 128 MiB. An input that cannot be
    /// loaded is `LoadFailed` (exit 100, D-01); a folder is `InvalidArgument`.
    pub fn quick(&self, req: &QuickRequest, job: &JobControl) -> Result<QuickResponse, ApiError> {
        run_guarded(|| self.quick_inner(req, job))
    }

    fn quick_inner(&self, req: &QuickRequest, job: &JobControl) -> Result<QuickResponse, ApiError> {
        let left = self.parse_spec(&req.left)?;
        let right = self.parse_spec(&req.right)?;
        let left_entry = self.stat_input(Side::Left, &left)?;
        let right_entry = self.stat_input(Side::Right, &right)?;
        if left_entry.kind == EntryKind::Dir || right_entry.kind == EntryKind::Dir {
            return Err(ApiError::InvalidArgument(
                "quick compares files; use `folder compare` for folders".into(),
            ));
        }
        let ctl = job.ctl();
        let vfs: &dyn Vfs = &**self.router();
        let stream = self.stream_options();
        let (format, outcome) = match req.method {
            QuickMethod::Size => {
                job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                let result = if left_entry.size == right_entry.size {
                    ContentResult::BinarySame
                } else {
                    ContentResult::BinaryDifferent
                };
                let outcome = ContentOutcome {
                    method: ContentMethod::None,
                    result,
                    warnings: Vec::new(),
                };
                (None, outcome)
            }
            QuickMethod::Crc | QuickMethod::Binary => {
                let l = VfsSource::new(vfs, left.clone(), left_entry);
                let r = VfsSource::new(vfs, right.clone(), right_entry);
                let outcome = if req.method == QuickMethod::Crc {
                    CrcComparator { opts: stream }.compare(&l, &r, &ctl)
                } else {
                    BinaryComparator { opts: stream }.compare(&l, &r, &ctl)
                }
                .map_err(|Cancelled| ApiError::Cancelled)?;
                (None, outcome)
            }
            QuickMethod::Rules => {
                let l = VfsSource::new(vfs, left.clone(), left_entry);
                let r = VfsSource::new(vfs, right.clone(), right_entry);
                let (choice, outcome) = self.quick_rules(req, &l, &r, job)?;
                (Some(choice), outcome)
            }
        };
        let exit_code = ExitCode::for_content(&outcome.result, req.ignore_unimportant);
        Ok(QuickResponse {
            left,
            right,
            method: req.method,
            format,
            outcome,
            ignore_unimportant: req.ignore_unimportant,
            exit_code,
        })
    }

    /// The Rules method of `quick` (§3.1 step 3, §2.19).
    fn quick_rules(
        &self,
        req: &QuickRequest,
        left: &VfsSource<'_>,
        right: &VfsSource<'_>,
        job: &JobControl,
    ) -> Result<(FormatChoice, ContentOutcome), ApiError> {
        let ctl = job.ctl();
        let mut warnings = Vec::new();
        let rules = self.text_rules(req.rules_file.as_deref(), &req.rules_patch, &mut warnings)?;
        let names = Pair::new(
            self.detection_name(left.spec()),
            self.detection_name(right.spec()),
        );
        let chosen =
            self.choose_format(req.format.as_deref(), names.as_ref().map(Option::as_deref))?;
        warnings.extend(chosen.warnings.iter().cloned());
        if !matches!(chosen.format.kind(), FormatKind::Text | FormatKind::Table) {
            let result = match compare_sources(left, right, &self.stream_options(), &ctl) {
                Ok(BinaryOutcome::Same) => ContentResult::BinarySame,
                Ok(BinaryOutcome::Different { .. }) => ContentResult::BinaryDifferent,
                Err(e) if e.is_cancelled() => return Err(ApiError::Cancelled),
                Err(e) => return Err(ApiError::from_vfs(e)),
            };
            let outcome = ContentOutcome {
                method: ContentMethod::Rules,
                result,
                warnings,
            };
            return Ok((chosen.choice, outcome));
        }
        let left_bytes = read_input(Side::Left, left, job)?;
        let right_bytes = read_input(Side::Right, right, job)?;
        let load = self.load_options(None)?;
        let external = self.external();
        let options = QuickOptions {
            limits: *self.registry().limits(),
            external: Some(external),
        };
        let left_label = names.left.unwrap_or_else(|| left.spec().canonical());
        let right_label = names.right.unwrap_or_else(|| right.spec().canonical());
        let mut outcome = quick_compare_bytes_with(
            (&left_bytes, &left_label),
            (&right_bytes, &right_label),
            &chosen.format,
            &rules,
            &load,
            &options,
            &ctl,
        )
        .map_err(|Cancelled| ApiError::Cancelled)?;
        drop_large(left_bytes, right_bytes);
        warnings.append(&mut outcome.warnings);
        outcome.warnings = warnings;
        Ok((chosen.choice, outcome))
    }

    /// Text compare (§3.2). `observer` receives `loaded`, phases and partial runs (GUI first screen).
    ///
    /// Steps: specs; rules (defaults ← `rules_file` ← `rules_patch`, validated with the registry's limits); the
    /// effective format (D-15) with the per-side detections and `format_mismatch`; both documents loaded on
    /// the service pool (`load_document` with the per-side encoding override, the locale fallback and the
    /// conversion runner); `observer.loaded`; `bytes_identical` in 1 MiB steps, the raw buffers freed (off the
    /// thread when large); `compare_with_limits` on the pool; the verdict and exit code under
    /// `ignore_unimportant` (§2.21).
    pub fn text_compare(
        &self,
        req: &TextCompareRequest,
        job: &JobControl,
        observer: Option<&mut dyn CompareObserver>,
    ) -> Result<TextCompareResponse, ApiError> {
        run_guarded(|| self.text_compare_inner(req, job, observer))
    }

    fn text_compare_inner(
        &self,
        req: &TextCompareRequest,
        job: &JobControl,
        mut observer: Option<&mut dyn CompareObserver>,
    ) -> Result<TextCompareResponse, ApiError> {
        let left = self.parse_spec(&req.left)?;
        let right = self.parse_spec(&req.right)?;
        let mut warnings = Vec::new();
        let rules = self.text_rules(req.rules_file.as_deref(), &req.rules_patch, &mut warnings)?;
        let names = Pair::new(self.detection_name(&left), self.detection_name(&right));
        let chosen =
            self.choose_format(req.format.as_deref(), names.as_ref().map(Option::as_deref))?;
        warnings.extend(chosen.warnings.iter().cloned());
        let load = Pair::new(
            self.load_options(req.encodings.left.as_deref())?,
            self.load_options(req.encodings.right.as_deref())?,
        );
        let ctl = job.ctl();
        let vfs: &dyn Vfs = &**self.router();
        let external = Some(self.external());
        let format = &chosen.format;
        let (left_loaded, right_loaded) = self.pool().join(
            || load_document(vfs, &left, format, &load.left, external, &ctl),
            || load_document(vfs, &right, format, &load.right, external, &ctl),
        );
        let (left_doc, left_raw) =
            left_loaded.map_err(|e| ApiError::from_text(e, Side::Left, &left.canonical()))?;
        let (right_doc, right_raw) =
            right_loaded.map_err(|e| ApiError::from_text(e, Side::Right, &right.canonical()))?;
        let left_doc: Arc<TextDocument> = Arc::new(left_doc);
        let right_doc: Arc<TextDocument> = Arc::new(right_doc);
        if let Some(observer) = observer.as_deref_mut() {
            observer.loaded(&left_doc, &right_doc);
        }
        let identical = bytes_identical(&left_raw, &right_raw, job);
        drop_large(left_raw, right_raw);
        let bytes_identical = identical?;
        let input = CompareInput {
            left: left_doc,
            right: right_doc,
            format: Arc::clone(&chosen.format),
            rules,
            bytes_identical,
            force_tokenize: false,
        };
        let comparison = compare_with_limits(
            input,
            self.registry().limits(),
            Some(self.pool()),
            &ctl,
            observer,
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, &left.canonical()))?;
        warnings.extend(comparison.warnings().iter().cloned());
        let verdict = comparison.verdict(req.ignore_unimportant);
        let exit_code = ExitCode::for_content(&verdict, req.ignore_unimportant);
        Ok(TextCompareResponse {
            comparison: Arc::new(comparison),
            format: chosen.choice,
            detection: chosen.detection,
            verdict,
            ignore_unimportant: req.ignore_unimportant,
            exit_code,
            warnings,
        })
    }

    /// Folder compare, step 1 (no I/O on the bases): parses specs and rules and creates the live model, which
    /// the GUI can render before and during the scan.
    ///
    /// Rules precedence (§3.5): defaults ← `rules_file` ← `Criteria::parse_list(criteria)` ← `patch`. The text
    /// rules of the injected comparator are built only for content method Rules.
    pub fn folder_prepare(
        &self,
        req: &FolderCompareRequest,
    ) -> Result<PreparedFolderCompare, ApiError> {
        run_guarded(|| self.folder_prepare_inner(req))
    }

    fn folder_prepare_inner(
        &self,
        req: &FolderCompareRequest,
    ) -> Result<PreparedFolderCompare, ApiError> {
        let left = self.parse_spec(&req.left)?;
        let right = self.parse_spec(&req.right)?;
        let mut warnings = Vec::new();
        let effective = self.folder_rules(req, &mut warnings)?;
        let model = FolderModel::new(left.clone(), right.clone(), effective.rules.clone())
            .map_err(ApiError::from_folder)?;
        Ok(PreparedFolderCompare {
            left,
            right,
            rules: effective.rules,
            text_rules: effective.text_rules,
            ignore_unimportant: effective.ignore_unimportant,
            model,
            warnings,
        })
    }

    /// The effective rules of a folder comparison (§3.5): defaults ← `rules_file` ←
    /// `Criteria::parse_list(criteria)` ← `patch`; the text rules of the injected comparator only for content
    /// method Rules (otherwise a given `--text-rules-file` or text patch is reported with `switch_ignored`); the
    /// effective ignore-unimportant (the request's flag or the criteria word).
    pub(crate) fn folder_rules(
        &self,
        req: &FolderCompareRequest,
        warnings: &mut Vec<Warning>,
    ) -> Result<EffectiveFolderRules, ApiError> {
        let mut rules = match req.rules_file.as_deref() {
            Some(file) => {
                let (spec, text) = self.read_text_file(file)?;
                let (rules, mut file_warnings) = FolderRules::from_toml(&text)
                    .map_err(|e| ApiError::in_rules_file(&spec, ApiError::from_folder(e)))?;
                for warning in &mut file_warnings {
                    warning.spec.get_or_insert_with(|| spec.canonical());
                }
                warnings.append(&mut file_warnings);
                rules
            }
            None => FolderRules::default(),
        };
        let mut ignore_unimportant = req.ignore_unimportant;
        if let Some(criteria) = req.criteria.as_deref() {
            let parsed = Criteria::parse_list(criteria).map_err(ApiError::from_folder)?;
            rules.apply(&parsed.patch).map_err(ApiError::from_folder)?;
            ignore_unimportant |= parsed.ignore_unimportant;
        }
        rules.apply(&req.patch).map_err(ApiError::from_folder)?;
        let text_rules = if rules.criteria.content.method == ContentMethod::Rules {
            Some(self.text_rules(req.text_rules_file.as_deref(), &req.text_patch, warnings)?)
        } else {
            if req.text_rules_file.is_some() || req.text_patch != TextRulesPatch::default() {
                warnings.push(Warning {
                    code: WarningCode::SwitchIgnored,
                    message: format!(
                        "the text rules (--text-rules-file, --unimportant…) apply only to content method \
                         rules; the content method is {}, so they are ignored",
                        rules.criteria.content.method.as_str()
                    ),
                    spec: None,
                });
            }
            None
        };
        Ok(EffectiveFolderRules {
            rules,
            text_rules,
            ignore_unimportant,
        })
    }

    /// Folder compare, step 2: scans into the prepared model (§3.3).
    ///
    /// The content engines are the hikasa-vfs CRC and binary comparators with [`Service::stream_options`]
    /// (`min(4, pool size)` mapped threads) and, for method Rules, hikasa-text's `RulesComparator` (the registry,
    /// the prepared text rules, the locale fallback, the 128 MiB per-file limit, the conversion runner and the
    /// same stream options), injected as a `ContentComparator`. The scan runs on the service pool with the
    /// clock's "now" and the content-test budget of `options.toml` (D-25). Cancellation after the bases are
    /// loaded returns `Ok` with an incomplete comparison (verdict `Incomplete`, exit 100).
    pub fn folder_run(
        &self,
        prepared: &PreparedFolderCompare,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<FolderCompareResponse, ApiError> {
        run_guarded(|| self.folder_run_inner(prepared, job, observer))
    }

    fn folder_run_inner(
        &self,
        prepared: &PreparedFolderCompare,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<FolderCompareResponse, ApiError> {
        let stream = self.stream_options();
        let rules_comparator = match &prepared.text_rules {
            Some(text_rules) => {
                let comparator = RulesComparator::new(
                    Arc::clone(self.registry()),
                    text_rules.clone(),
                    self.load_options(None)?,
                )
                .map_err(|e| ApiError::from_text(e, Side::Left, ""))?
                .with_max_bytes(DEFAULT_RULES_MAX_BYTES)
                .with_external(self.env().temp_dir.clone(), Arc::clone(self.runner()))
                .with_stream_options(stream);
                Some(Arc::new(comparator) as Arc<dyn ContentComparator>)
            }
            None => None,
        };
        let engines = ContentEngines::with_stream_options(stream, rules_comparator);
        let now = self.clock().now();
        let env = FolderEnv {
            vfs: &**self.router(),
            content: &engines,
            pool: Some(self.pool()),
            ctl: job.ctl(),
            observer,
            now: &now,
            content_budget: self.options().content_budget_bytes(),
        };
        let comparison = compare_into(&prepared.model, &env).map_err(ApiError::from_folder)?;
        let mut warnings = prepared.warnings.clone();
        warnings.extend(comparison.read().warnings().iter().cloned());
        let verdict = comparison.verdict(prepared.ignore_unimportant);
        Ok(FolderCompareResponse {
            comparison,
            verdict,
            ignore_unimportant: prepared.ignore_unimportant,
            exit_code: ExitCode::for_verdict(verdict),
            warnings,
        })
    }

    /// `folder_prepare` + `folder_run` (CLI).
    pub fn folder_compare(
        &self,
        req: &FolderCompareRequest,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<FolderCompareResponse, ApiError> {
        let prepared = self.folder_prepare(req)?;
        self.folder_run(&prepared, job, observer)
    }

    // ---- shared helpers -------------------------------------------------------------------------------------

    /// Stats an input of a file comparison (`LoadFailed` on failure).
    fn stat_input(&self, side: Side, spec: &Spec) -> Result<Entry, ApiError> {
        self.router()
            .stat(spec, FollowLinks::Yes)
            .map_err(|source| {
                if source.is_cancelled() {
                    ApiError::Cancelled
                } else {
                    ApiError::LoadFailed {
                        side,
                        spec: spec.canonical(),
                        source,
                    }
                }
            })
    }

    /// The name used for format detection (D-15): the spec's file name, or the stdin/clipboard name hint.
    pub(crate) fn detection_name(&self, spec: &Spec) -> Option<String> {
        let hint = match spec {
            Spec::Stdin => self.stdin_name().map(str::to_owned),
            _ => self.router().name_hint(spec).map(str::to_owned),
        };
        spec.file_name().or(hint).filter(|name| !name.is_empty())
    }

    /// The effective format (D-15): `forced` (§2.12.2), else the detection of the first named side (left, then
    /// right); the other named side is detected for information and a `format_mismatch` warning.
    pub(crate) fn choose_format(
        &self,
        forced: Option<&str>,
        names: Pair<Option<&str>>,
    ) -> Result<ChosenFormat, ApiError> {
        let registry = self.registry();
        let overrides = EnabledOverrides::default();
        let detect = |name: &str| registry.detect(name, &overrides);
        let mut warnings = Vec::new();
        if let Some(forced) = forced {
            let detection = registry.force(forced)?;
            let informational =
                names.map(|name| name.and_then(|n| detect(n).ok()).map(|d| d.reason));
            return Ok(chosen(detection, informational, warnings));
        }
        match (names.left, names.right) {
            (Some(l), right_name) => {
                let left_detection = detect(l)?;
                let right_reason = match right_name {
                    Some(r) => match detect(r) {
                        Ok(right_detection) => {
                            if right_detection.format.id() != left_detection.format.id() {
                                warnings.push(mismatch_warning(
                                    l,
                                    r,
                                    &left_detection.format,
                                    &right_detection.format,
                                ));
                            }
                            Some(right_detection.reason)
                        }
                        Err(e) => {
                            warnings.push(Warning {
                                code: WarningCode::FormatMismatch,
                                message: format!(
                                    "{r}: its format could not be detected ({e}); {} (the format of {l}) is \
                                     used for both (D-15)",
                                    left_detection.format.name()
                                ),
                                spec: None,
                            });
                            None
                        }
                    },
                    None => None,
                };
                let left_reason = Some(left_detection.reason.clone());
                Ok(chosen(
                    left_detection,
                    Pair::new(left_reason, right_reason),
                    warnings,
                ))
            }
            (None, Some(r)) => {
                let detection = detect(r)?;
                let right_reason = Some(detection.reason.clone());
                Ok(chosen(detection, Pair::new(None, right_reason), warnings))
            }
            (None, None) => {
                let detection = Detection {
                    format: Arc::clone(registry.everything_else()),
                    reason: DetectReason::Fallback,
                };
                Ok(chosen(detection, Pair::new(None, None), warnings))
            }
        }
    }

    /// Document load options: the per-pane encoding override (an unknown label is `UnknownEncoding`), the
    /// locale fallback, the format's characters-per-line limit and the D-43 size cap.
    pub(crate) fn load_options(&self, encoding: Option<&str>) -> Result<LoadOptions, ApiError> {
        let mut load = LoadOptions::default();
        load.decode.locale_fallback = self.locale_encoding();
        if let Some(label) = encoding {
            let encoding = TextEncoding::for_label(label).ok_or_else(|| {
                ApiError::Format(FormatError::UnknownEncoding {
                    label: label.to_owned(),
                })
            })?;
            load.decode.override_encoding = Some(encoding);
        }
        load.max_bytes = MAX_DOCUMENT_BYTES;
        Ok(load)
    }

    /// Text rules: defaults ← the rules file (unknown keys → warnings naming the file) ← the patch, validated
    /// with the registry's regex limits.
    pub(crate) fn text_rules(
        &self,
        file: Option<&OsStr>,
        patch: &TextRulesPatch,
        warnings: &mut Vec<Warning>,
    ) -> Result<TextRules, ApiError> {
        let mut rules = match file {
            Some(file) => {
                let (spec, text) = self.read_text_file(file)?;
                let (rules, mut file_warnings) = TextRules::from_toml(&text).map_err(|e| {
                    ApiError::in_rules_file(&spec, ApiError::from_text(e, Side::Left, ""))
                })?;
                for warning in &mut file_warnings {
                    warning.spec.get_or_insert_with(|| spec.canonical());
                }
                warnings.append(&mut file_warnings);
                rules
            }
            None => TextRules::default(),
        };
        rules.apply(patch);
        rules
            .validate(self.registry().limits())
            .map_err(|e| ApiError::from_text(e, Side::Left, ""))?;
        Ok(rules)
    }

    /// Reads a UTF-8 text file named by a spec (rules files).
    pub(crate) fn read_text_file(&self, file: &OsStr) -> Result<(Spec, String), ApiError> {
        let spec = self.parse_spec(file)?;
        let mut reader = self.router().open_read(&spec).map_err(ApiError::from_vfs)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut bytes)
            .map_err(|e| ApiError::from_vfs(VfsError::from_io(spec.canonical(), VfsOp::Read, e)))?;
        let text = String::from_utf8(bytes).map_err(|_| {
            ApiError::InvalidArgument(format!("{}: the file is not valid UTF-8", spec.canonical()))
        })?;
        Ok((spec, text))
    }
}

/// Builds a [`ChosenFormat`] from a detection.
fn chosen(
    detection: Detection,
    per_side: Pair<Option<DetectReason>>,
    warnings: Vec<Warning>,
) -> ChosenFormat {
    let format = detection.format;
    let choice = FormatChoice {
        id: format.id().cloned(),
        name: format.name().to_owned(),
        kind: format.kind(),
        reason: detection.reason,
    };
    ChosenFormat {
        format,
        choice,
        detection: per_side,
        warnings,
    }
}

/// The `format_mismatch` warning of D-15 (same wording as `RulesComparator`).
fn mismatch_warning(
    left: &str,
    right: &str,
    left_format: &CompiledFormat,
    right_format: &CompiledFormat,
) -> Warning {
    Warning {
        code: WarningCode::FormatMismatch,
        message: format!(
            "{left} is detected as {}, {right} as {}; {} is used for both (D-15)",
            left_format.name(),
            right_format.name(),
            left_format.name()
        ),
        spec: None,
    }
}

/// Reads a quick-compare input completely (the D-43 document limit; cancellation checked per 1 MiB).
fn read_input(side: Side, source: &VfsSource<'_>, job: &JobControl) -> Result<Vec<u8>, ApiError> {
    source
        .read_all(MAX_DOCUMENT_BYTES, &job.cancel)
        .map_err(|e| match e {
            ReadError::Cancelled => ApiError::Cancelled,
            ReadError::TooLarge { limit } => ApiError::Text(TextError::TooLarge {
                spec: source.spec().canonical(),
                limit,
            }),
            ReadError::Io(e) => {
                let source_error = VfsError::from_io(source.spec().canonical(), VfsOp::Read, e);
                if source_error.is_cancelled() {
                    ApiError::Cancelled
                } else {
                    ApiError::LoadFailed {
                        side,
                        spec: source.spec().canonical(),
                        source: source_error,
                    }
                }
            }
            other => ApiError::Internal(format!("unexpected read error: {other}")),
        })
}

/// `left == right`, compared in 1 MiB steps with a cancellation check before each step (§3.2 step 3).
fn bytes_identical(left: &[u8], right: &[u8], job: &JobControl) -> Result<bool, ApiError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left.chunks(COMPARE_STEP).zip(right.chunks(COMPARE_STEP)) {
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Frees two raw buffers, on a short-lived thread when they are large (freeing hundreds of MiB can take tens of
/// milliseconds); on the calling thread when the thread cannot start.
fn drop_large(left: Vec<u8>, right: Vec<u8>) {
    if left.capacity() + right.capacity() < DROP_OFF_THREAD_BYTES {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hikasa-free".into())
        .spawn(move || drop((left, right)));
    if let Err(e) = spawned {
        tracing::debug!(target: "hikasa::service", "freeing on the calling thread: {e}");
    }
}
