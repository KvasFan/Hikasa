//! `quick`, `text compare` and `folder compare` (§3.1–§3.3).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::sync::Arc;

use hikasa_core::{
    CancelToken, Cancelled, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, ExitCode, Pair, ReadError, Side, Spec, Warning, WarningCode,
};
use hikasa_folder::sync::{SyncOptions, sync_folder_rules};
use hikasa_folder::{
    ArchiveHandling, ContentEngines, Criteria, FolderEnv, FolderModel, FolderRules, ScanObserver,
    compare_into,
};
use hikasa_formats::{
    CompiledFormat, DetectReason, Detection, EnabledOverrides, FormatError, FormatId, FormatKind,
    TextEncoding,
};
use hikasa_text::{
    Anchor, AnchorSide, CompareInput, CompareObserver, DEFAULT_RULES_MAX_BYTES, IgnoredLines,
    LoadOptions, MAX_DOCUMENT_BYTES, QuickOptions, RangeSet, RulesComparator, TextComparison,
    TextDocument, TextError, TextRules, TextRulesPatch, compare_with_limits, ignored_check,
    load_document, quick_compare_bytes_with,
};
use hikasa_vfs::{
    BinaryComparator, BinaryOutcome, CrcComparator, Entry, EntryKind, FollowLinks, SpecUse, Vfs,
    VfsError, VfsOp, VfsSource, compare_sources,
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
    /// FF-002: the formats this comparison turns on or off.
    pub(crate) overrides: EnabledOverrides,
}

/// The effective format of a file comparison with its per-side detections and warnings (D-15).
#[derive(Clone)]
pub(crate) struct ChosenFormat {
    pub(crate) format: Arc<CompiledFormat>,
    pub(crate) choice: FormatChoice,
    pub(crate) detection: Pair<Option<DetectReason>>,
    pub(crate) warnings: Vec<Warning>,
}

/// What a failed input of a file comparison is.
pub(crate) fn load_failed(side: Side, spec: &Spec, source: VfsError) -> ApiError {
    if source.is_cancelled() {
        ApiError::Cancelled
    } else {
        ApiError::LoadFailed {
            side,
            spec: spec.canonical(),
            source,
        }
    }
}

/// Both documents of a text comparison, loaded ([`Service::load_text_pair`]).
pub(crate) struct LoadedPair {
    /// The resolved specs.
    pub(crate) specs: Pair<Spec>,
    /// The documents.
    pub(crate) docs: Pair<Arc<TextDocument>>,
    /// The effective format (of both sides, or of the left one when the right one has its own).
    pub(crate) chosen: ChosenFormat,
    /// The right side's own format when it differs from the left one (D-281, §1.10.16).
    pub(crate) right_chosen: Option<ChosenFormat>,
    /// The ignored lines, checked against the documents (§1.10.17).
    pub(crate) ignored: IgnoredLines,
    /// The effective rules.
    pub(crate) rules: TextRules,
    /// The raw bytes were identical.
    pub(crate) bytes_identical: bool,
    /// Warnings so far (rules file, `format_mismatch`).
    pub(crate) warnings: Vec<Warning>,
}

impl LoadedPair {
    /// The engine's input, with the request's anchors.
    pub(crate) fn compare_input(&self, req: &TextCompareRequest) -> CompareInput {
        CompareInput {
            left: Arc::clone(&self.docs.left),
            right: Arc::clone(&self.docs.right),
            format: Arc::clone(&self.chosen.format),
            rules: self.rules.clone(),
            bytes_identical: self.bytes_identical,
            force_tokenize: false,
            anchors: req.anchors.clone(),
            right_format: self.right_chosen.as_ref().map(|c| Arc::clone(&c.format)),
            ignored: self.ignored.clone(),
        }
    }

    /// The response of a finished comparison: the warnings so far and the comparison's, the verdict and the exit
    /// code under the request's `ignore_unimportant` (§2.21).
    pub(crate) fn respond(
        self,
        req: &TextCompareRequest,
        comparison: Arc<TextComparison>,
    ) -> TextCompareResponse {
        let mut warnings = self.warnings;
        warnings.extend(comparison.warnings().iter().cloned());
        let verdict = comparison.verdict(req.ignore_unimportant);
        let exit_code = ExitCode::for_content(&verdict, req.ignore_unimportant);
        let check = |doc: &TextDocument, lines: &RangeSet| {
            (!lines.is_empty())
                .then(|| ignored_check(doc, lines))
                .flatten()
        };
        let ignored_check = Pair::new(
            check(&self.docs.left, &self.ignored.left),
            check(&self.docs.right, &self.ignored.right),
        );
        TextCompareResponse {
            comparison,
            right_format: self.right_chosen.map(|c| c.choice),
            ignored_check,
            format: self.chosen.choice,
            detection: self.chosen.detection,
            verdict,
            ignore_unimportant: req.ignore_unimportant,
            exit_code,
            warnings,
        }
    }
}

/// The request's ignored lines against the loaded documents (§1.10.17): a side with a saved check keeps its marks
/// only while its marked lines still have that check (else they are dropped with `ignored_lines_dropped`); a
/// side without one must name lines of its document (else `InvalidArgument`, exit 100).
pub(crate) fn check_ignored(
    req: &TextCompareRequest,
    docs: Pair<&TextDocument>,
    specs: Pair<&Spec>,
    warnings: &mut Vec<Warning>,
) -> Result<IgnoredLines, ApiError> {
    let mut ignored = req.ignored.clone();
    for (side, doc, spec, check) in [
        (Side::Left, docs.left, specs.left, req.ignored_check.left),
        (
            Side::Right,
            docs.right,
            specs.right,
            req.ignored_check.right,
        ),
    ] {
        let lines = if side == Side::Right {
            &mut ignored.right
        } else {
            &mut ignored.left
        };
        if lines.is_empty() {
            continue;
        }
        match check {
            Some(saved) => {
                if ignored_check(doc, lines) != Some(saved) {
                    warnings.push(Warning {
                        code: WarningCode::IgnoredLinesDropped,
                        message: format!(
                            "the ignored lines of the {side} file were dropped: its marked lines changed since \
                             they were saved ({} lines)",
                            lines.line_count()
                        ),
                        spec: Some(spec.canonical()),
                    });
                    *lines = RangeSet::new();
                }
            }
            None => {
                if let Some(last) = lines.ranges().last()
                    && last.end > doc.line_count()
                {
                    return Err(ApiError::InvalidArgument(format!(
                        "--ignored {side}:{}-{}: line {} is beyond the {side} document ({} lines)",
                        last.start + 1,
                        last.end,
                        last.end,
                        doc.line_count()
                    )));
                }
            }
        }
    }
    Ok(ignored)
}

/// The request's anchors against the loaded documents (§1.10.8): every line an anchor names must exist, else
/// `InvalidArgument` (exit 100) naming the anchor and the document's line count. Crossing and overlapping pairs
/// are left to the engine, which drops them with `anchor_dropped`.
pub(crate) fn check_anchors(
    anchors: &[Anchor],
    left: &TextDocument,
    right: &TextDocument,
) -> Result<(), ApiError> {
    let lines = |side: AnchorSide| match side {
        AnchorSide::Left => left.line_count(),
        AnchorSide::Right => right.line_count(),
    };
    let beyond = |anchor: &Anchor, side: AnchorSide, end: u32| {
        let name = match side {
            AnchorSide::Left => "left",
            AnchorSide::Right => "right",
        };
        ApiError::InvalidArgument(format!(
            "anchor `{anchor}`: line {end} is beyond the {name} document ({} lines)",
            lines(side)
        ))
    };
    for anchor in anchors {
        match anchor {
            Anchor::Pair { left, right } => {
                if left.end > lines(AnchorSide::Left) {
                    return Err(beyond(anchor, AnchorSide::Left, left.end));
                }
                if right.end > lines(AnchorSide::Right) {
                    return Err(beyond(anchor, AnchorSide::Right, right.end));
                }
            }
            Anchor::Isolate { side, lines: range } => {
                if range.end > lines(*side) {
                    return Err(beyond(anchor, *side, range.end));
                }
            }
        }
    }
    Ok(())
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
        let left = self.parse_spec_in(&req.left, self.use_of(&req.left), &job.cancel)?;
        let right = self.parse_spec_in(&req.right, self.use_of(&req.right), &job.cancel)?;
        let left_entry = self.stat_input(Side::Left, &left, job)?;
        let right_entry = self.stat_input(Side::Right, &right, job)?;
        if left_entry.kind == EntryKind::Dir || right_entry.kind == EntryKind::Dir {
            return Err(ApiError::InvalidArgument(
                "quick compares files; use `folder compare` for folders".into(),
            ));
        }
        let ctl = job.ctl();
        let router = self.router();
        let vfs: &dyn Vfs = &*router;
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
                let l = VfsSource::new(vfs, left.clone(), left_entry).with_cancel(&job.cancel);
                let r = VfsSource::new(vfs, right.clone(), right_entry).with_cancel(&job.cancel);
                let outcome = if req.method == QuickMethod::Crc {
                    CrcComparator { opts: stream }.compare(&l, &r, &ctl)
                } else {
                    BinaryComparator { opts: stream }.compare(&l, &r, &ctl)
                }
                .map_err(|Cancelled| ApiError::Cancelled)?;
                (None, outcome)
            }
            QuickMethod::Rules => {
                let l = VfsSource::new(vfs, left.clone(), left_entry).with_cancel(&job.cancel);
                let r = VfsSource::new(vfs, right.clone(), right_entry).with_cancel(&job.cancel);
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
        if chosen.format.kind() == FormatKind::Table {
            // T-21, §6.5: a table pair compares through the table engine.
            let comparator = super::table::table_comparator(
                self.registry(),
                EnabledOverrides::default(),
                self.system_regional(),
            );
            let mut outcome = self
                .pool()
                .install(|| comparator.compare(left, right, &ctl))
                .map_err(|Cancelled| ApiError::Cancelled)?;
            warnings.append(&mut outcome.warnings);
            outcome.warnings = warnings;
            return Ok((chosen.choice, outcome));
        }
        if chosen.format.kind() == FormatKind::Picture {
            // D-344: a picture pair compares by its pixels with the Picture Compare defaults.
            let comparator = self.picture_comparator();
            let mut outcome = self
                .pool()
                .install(|| comparator.compare(left, right, &ctl))
                .map_err(|Cancelled| ApiError::Cancelled)?;
            warnings.append(&mut outcome.warnings);
            outcome.warnings = warnings;
            return Ok((chosen.choice, outcome));
        }
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
        let loaded = self.load_text_pair(None, req, job, &mut observer)?;
        let ctl = job.ctl();
        let comparison = compare_with_limits(
            loaded.compare_input(req),
            self.registry().limits(),
            Some(self.pool()),
            &ctl,
            observer,
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, &loaded.specs.left.canonical()))?;
        Ok(loaded.respond(req, Arc::new(comparison)))
    }

    /// An input spec of a file comparison: through the router, or (`on`) resolved on the back-end `vfs` (a test's
    /// `MemoryVfs`; standard input and the clipboard are still captured by the service).
    pub(crate) fn parse_input(
        &self,
        on: Option<&dyn Vfs>,
        raw: &OsStr,
        cancel: &CancelToken,
    ) -> Result<Spec, ApiError> {
        match on {
            None => self.parse_spec_in(raw, self.use_of(raw), cancel),
            Some(vfs) => {
                let spec = Spec::parse_os(raw, &self.spec_context())?;
                self.capture_input(&spec)?;
                hikasa_vfs::resolve_spec(vfs, spec, self.use_of(raw), cancel)
                    .map_err(|_| ApiError::Cancelled)
            }
        }
    }

    /// The first steps of a text comparison (§3.2), shared by [`Service::text_compare`] and
    /// [`Service::text_compare_live`]: specs; rules; the effective format (D-15); both documents loaded on the pool
    /// (from `on`, else the router); `observer.loaded`; the anchors checked against the documents (a line beyond a
    /// document is `InvalidArgument`, exit 100, §1.10.8); `bytes_identical`.
    pub(crate) fn load_text_pair(
        &self,
        on: Option<&dyn Vfs>,
        req: &TextCompareRequest,
        job: &JobControl,
        observer: &mut Option<&mut dyn CompareObserver>,
    ) -> Result<LoadedPair, ApiError> {
        let left = self.parse_input(on, &req.left, &job.cancel)?;
        let right = self.parse_input(on, &req.right, &job.cancel)?;
        let router = self.router();
        let vfs: &dyn Vfs = on.unwrap_or(&*router);
        for (side, spec) in [(Side::Left, &left), (Side::Right, &right)] {
            vfs.prepare(spec, &job.ctl())
                .map_err(|source| load_failed(side, spec, source))?;
        }
        let mut warnings = Vec::new();
        let rules = self.text_rules_from(
            req.base_rules.as_ref(),
            req.rules_file.as_deref(),
            &req.rules_patch,
            &mut warnings,
        )?;
        let names = Pair::new(self.detection_name(&left), self.detection_name(&right));
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let base = self.choose_format_with(
            req.format.as_deref(),
            names.as_ref().map(Option::as_deref),
            &overrides,
        )?;
        let (chosen, right_chosen) = self.per_side_formats(req, base)?;
        warnings.extend(chosen.warnings.iter().cloned());
        let load = Pair::new(
            self.load_options(req.encodings.left.as_deref())?,
            self.load_options(req.encodings.right.as_deref())?,
        );
        let ctl = job.ctl();
        let external = Some(self.external());
        let format = &chosen.format;
        // Each document is read with its own side's format (conversion, wrapping, encoding fallback).
        let right_format = right_chosen.as_ref().map_or(format, |c| &c.format);
        let (left_loaded, right_loaded) = self.pool().join(
            || load_document(vfs, &left, format, &load.left, external, &ctl),
            || load_document(vfs, &right, right_format, &load.right, external, &ctl),
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
        check_anchors(&req.anchors, &left_doc, &right_doc)?;
        let ignored = check_ignored(
            req,
            Pair::new(&left_doc, &right_doc),
            Pair::new(&left, &right),
            &mut warnings,
        )?;
        Ok(LoadedPair {
            specs: Pair::new(left, right),
            docs: Pair::new(left_doc, right_doc),
            chosen,
            right_chosen,
            ignored,
            rules,
            bytes_identical,
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
        run_guarded(|| self.folder_prepare_inner(req, &CancelToken::never()))
    }

    /// [`Service::folder_prepare`] inside a job: a base inside a nested archive is resolved by indexing the archive
    /// around it, and that stops with the job (§1.5.4).
    pub fn folder_prepare_in(
        &self,
        req: &FolderCompareRequest,
        job: &JobControl,
    ) -> Result<PreparedFolderCompare, ApiError> {
        run_guarded(|| self.folder_prepare_inner(req, &job.cancel))
    }

    /// The comparison of a sync (§1.6.14): [`Service::folder_prepare_in`] with the rules `sync_folder_rules` makes of the
    /// request's (every folder scanned, archives by *Sync within archives*, flatten refused). The sync's words are checked
    /// here, before anything is scanned.
    pub fn folder_sync_prepare(
        &self,
        req: &FolderCompareRequest,
        sync: &SyncOptions,
        job: &JobControl,
    ) -> Result<PreparedFolderCompare, ApiError> {
        run_guarded(|| {
            sync.validate().map_err(ApiError::from_folder)?;
            self.folder_prepare_with(req, &job.cancel, Some(sync))
        })
    }

    fn folder_prepare_inner(
        &self,
        req: &FolderCompareRequest,
        cancel: &CancelToken,
    ) -> Result<PreparedFolderCompare, ApiError> {
        self.folder_prepare_with(req, cancel, None)
    }

    fn folder_prepare_with(
        &self,
        req: &FolderCompareRequest,
        cancel: &CancelToken,
        sync: Option<&SyncOptions>,
    ) -> Result<PreparedFolderCompare, ApiError> {
        // A base is a folder: `a.zip` is the archive's root, whatever the handling mode (FC-001).
        let left = self.parse_spec_in(&req.left, SpecUse::Folder, cancel)?;
        let right = self.parse_spec_in(&req.right, SpecUse::Folder, cancel)?;
        let mut warnings = Vec::new();
        let mut effective = self.folder_rules(req, &mut warnings)?;
        if let Some(sync) = sync {
            effective.rules =
                sync_folder_rules(&effective.rules, sync).map_err(ApiError::from_folder)?;
        }
        let model = FolderModel::new(left.clone(), right.clone(), effective.rules.clone())
            .map_err(ApiError::from_folder)?;
        Ok(PreparedFolderCompare {
            left,
            right,
            rules: effective.rules,
            text_rules: effective.text_rules,
            overrides: effective.overrides,
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
            None => req.base_rules.clone().unwrap_or_default(),
        };
        let mut ignore_unimportant = req.ignore_unimportant;
        if let Some(criteria) = req.criteria.as_deref() {
            let parsed = Criteria::parse_list(criteria).map_err(ApiError::from_folder)?;
            rules.apply(&parsed.patch).map_err(ApiError::from_folder)?;
            ignore_unimportant |= parsed.ignore_unimportant;
        }
        rules.apply(&req.patch).map_err(ApiError::from_folder)?;
        if rules.handling.archives == ArchiveHandling::AsFiles
            && req
                .patch
                .open_archives
                .as_ref()
                .is_some_and(|p| !p.is_empty())
        {
            warnings.push(Warning {
                code: WarningCode::SwitchIgnored,
                message: "--open-archive has no effect with --archives files".into(),
                spec: None,
            });
        }
        let text_rules = if rules.criteria.content.method == ContentMethod::Rules {
            Some(self.text_rules_from(
                req.base_text_rules.as_ref(),
                req.text_rules_file.as_deref(),
                &req.text_patch,
                warnings,
            )?)
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
        let overrides = self.enabled_overrides(&rules.format_overrides, warnings);
        Ok(EffectiveFolderRules {
            rules,
            text_rules,
            ignore_unimportant,
            overrides,
        })
    }

    /// Checks folder rules as the comparison would take them (M2 W9c, D-249): the compile step of
    /// [`Service::folder_prepare`] without bases: the schema version, the name filters' masks, the time-zone offset
    /// (`FolderRules::validate`). `Ok` carries the warnings the comparison would give: an unknown format id of
    /// `format_overrides` (`unknown_config_key`), opened archives while archives are files (`switch_ignored`). The rule
    /// tabs' OK runs it in a job before anything is written.
    pub fn check_folder_rules(&self, rules: &FolderRules) -> Result<Vec<Warning>, ApiError> {
        run_guarded(|| {
            rules.validate().map_err(ApiError::from_folder)?;
            let mut warnings = Vec::new();
            if rules.handling.archives == ArchiveHandling::AsFiles
                && !rules.open_archives.is_empty()
            {
                warnings.push(Warning {
                    code: WarningCode::SwitchIgnored,
                    message: "the opened archives have no effect while archives are files".into(),
                    spec: None,
                });
            }
            let _ = self.enabled_overrides(&rules.format_overrides, &mut warnings);
            Ok(warnings)
        })
    }

    /// The overrides of FF-002, from plain data (D-172). An id the registry does not know gives one `unknown_config_key`
    /// warning and is kept (Q11): the format can come back, and then the session means it.
    pub(crate) fn enabled_overrides(
        &self,
        words: &BTreeMap<String, bool>,
        warnings: &mut Vec<Warning>,
    ) -> EnabledOverrides {
        let registry = self.registry();
        let mut overrides = BTreeMap::new();
        for (id, enabled) in words {
            let known = FormatId::new(id.clone())
                .ok()
                .inspect(|format| {
                    overrides.insert(format.clone(), *enabled);
                })
                .is_some_and(|format| registry.get(&format).is_some());
            if !known {
                warnings.push(Warning {
                    code: WarningCode::UnknownConfigKey,
                    message: format!("format_overrides: unknown format id `{id}`"),
                    spec: None,
                });
            }
        }
        EnabledOverrides(overrides)
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

    /// The content engines of a comparison: the hikasa-vfs CRC and binary comparators and, with `text_rules`, hikasa-text's
    /// `RulesComparator`.
    pub(crate) fn content_engines(
        &self,
        text_rules: Option<&TextRules>,
        overrides: &EnabledOverrides,
    ) -> Result<ContentEngines, ApiError> {
        let stream = self.stream_options();
        let rules_comparator = match text_rules {
            Some(text_rules) => {
                let comparator = RulesComparator::new(
                    self.registry(),
                    text_rules.clone(),
                    self.load_options(None)?,
                )
                .map_err(|e| ApiError::from_text(e, Side::Left, ""))?
                .with_overrides(overrides.clone())
                .with_max_bytes(DEFAULT_RULES_MAX_BYTES)
                .with_external(self.env().temp_dir.clone(), Arc::clone(self.runner()))
                .with_stream_options(stream);
                // T-21: pairs of a Table format go to the table engine.
                let dispatch = super::table::RulesDispatch::new(
                    Arc::new(comparator),
                    self.registry(),
                    overrides.clone(),
                    self.system_regional(),
                )
                .with_pictures(self.picture_comparator());
                Some(Arc::new(dispatch) as Arc<dyn ContentComparator>)
            }
            None => None,
        };
        Ok(ContentEngines::with_stream_options(
            stream,
            rules_comparator,
        ))
    }

    fn folder_run_inner(
        &self,
        prepared: &PreparedFolderCompare,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<FolderCompareResponse, ApiError> {
        let engines = self.content_engines(prepared.text_rules.as_ref(), &prepared.overrides)?;
        let now = self.clock().now();
        let router = self.router();
        let env = FolderEnv {
            vfs: &*router,
            content: &engines,
            pool: Some(self.pool()),
            ctl: job.ctl(),
            observer,
            now: &now,
            content_budget: self.options().content_budget_bytes(),
            archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
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
        let prepared = self.folder_prepare_in(req, job)?;
        self.folder_run(&prepared, job, observer)
    }

    // ---- shared helpers -------------------------------------------------------------------------------------

    /// Does the slow part of reaching an input inside the job (`prepare`): `stat` and a document load cannot be
    /// cancelled while they index an archive.
    fn prepare_input(&self, side: Side, spec: &Spec, job: &JobControl) -> Result<(), ApiError> {
        self.router()
            .prepare(spec, &job.ctl())
            .map_err(|source| load_failed(side, spec, source))
    }

    /// Stats an input of a file comparison (`LoadFailed` on failure).
    pub(crate) fn stat_input(
        &self,
        side: Side,
        spec: &Spec,
        job: &JobControl,
    ) -> Result<Entry, ApiError> {
        self.prepare_input(side, spec, job)?;
        self.router()
            .stat(spec, FollowLinks::Yes)
            .map_err(|source| load_failed(side, spec, source))
    }

    /// The name used for format detection (D-15): the spec's file name, or the stdin/clipboard name hint.
    pub(crate) fn detection_name(&self, spec: &Spec) -> Option<String> {
        let hint = match spec {
            Spec::Stdin => self.stdin_name().map(str::to_owned),
            _ => self.router().name_hint(spec).map(str::to_owned),
        };
        spec.file_name().or(hint).filter(|name| !name.is_empty())
    }

    /// A format per pane (D-281, §1.10.16): each side's own format (`req.formats`, forced by id or name) or else
    /// `base`, D-15's format of the pair; the right one is returned only when it differs from the left one.
    /// `--format` sets both sides and conflicts with either side's own (`InvalidArgument`, exit 100). With a
    /// side's own format, D-15's `format_mismatch` ("… is used for both") no longer applies and is dropped.
    pub(crate) fn per_side_formats(
        &self,
        req: &TextCompareRequest,
        base: ChosenFormat,
    ) -> Result<(ChosenFormat, Option<ChosenFormat>), ApiError> {
        if req.formats.left.is_none() && req.formats.right.is_none() {
            return Ok((base, None));
        }
        if req.format.is_some() {
            return Err(ApiError::InvalidArgument(
                "--format sets the format of both sides; it cannot be combined with --left-format or \
                 --right-format"
                    .into(),
            ));
        }
        // A pane can also be plain text: "Everything Else" (any case) or `everything-else`, the format no registry
        // entry stands for (its reason is `fallback`).
        let registry = self.registry();
        let side = |name: &Option<String>| -> Result<Option<ChosenFormat>, ApiError> {
            name.as_deref()
                .map(|n| {
                    let word = n.trim();
                    let detection = if word.eq_ignore_ascii_case("everything else")
                        || word == "everything-else"
                    {
                        Detection {
                            format: Arc::clone(registry.everything_else()),
                            reason: DetectReason::Fallback,
                        }
                    } else {
                        registry.force(word)?
                    };
                    Ok(chosen(detection, base.detection.clone(), Vec::new()))
                })
                .transpose()
        };
        let (left, right) = (side(&req.formats.left)?, side(&req.formats.right)?);
        let mut base = base;
        base.warnings
            .retain(|w| w.code != WarningCode::FormatMismatch);
        let left = left.unwrap_or_else(|| base.clone());
        let right = right.unwrap_or(base);
        let same = Arc::ptr_eq(&left.format, &right.format)
            || (left.format.id().is_some() && left.format.id() == right.format.id());
        Ok((left, (!same).then_some(right)))
    }

    /// The effective format (D-15): `forced` (§2.12.2), else the detection of the first named side (left, then
    /// right); the other named side is detected for information and a `format_mismatch` warning.
    pub(crate) fn choose_format(
        &self,
        forced: Option<&str>,
        names: Pair<Option<&str>>,
    ) -> Result<ChosenFormat, ApiError> {
        self.choose_format_with(forced, names, &EnabledOverrides::default())
    }

    /// [`Service::choose_format`] with a comparison's own overrides (FF-002).
    pub(crate) fn choose_format_with(
        &self,
        forced: Option<&str>,
        names: Pair<Option<&str>>,
        overrides: &EnabledOverrides,
    ) -> Result<ChosenFormat, ApiError> {
        let registry = self.registry();
        let detect = |name: &str| registry.detect(name, overrides);
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
                                     used for both",
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
        self.text_rules_from(None, file, patch, warnings)
    }

    /// [`Service::text_rules`] with the rules a missing file means (`None`: the defaults; M2 W2).
    pub(crate) fn text_rules_from(
        &self,
        base: Option<&TextRules>,
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
            None => base.cloned().unwrap_or_default(),
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
            "{left} is detected as {}, {right} as {}; {} is used for both",
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
