//! Rule and format changes that keep a view's edits (M2 W11a; contract §1.11.13, D-304): [`Service::text_relive`]
//! compares the live documents' snapshots again with a request's rules, per-side formats, anchors and ignored lines.
//! The documents are not re-read: their text, undo logs, marks and bookmarks stay the view's.
//!
//! ```text
//! text_relive(docs, req) = the steps of text_compare without the loading (load_text_pair):
//!                          rules (base_rules ← rules_file ← rules_patch), D-15's format of the pair from the specs'
//!                          names (or req.format), the per-side formats (req.formats), the anchors and ignored lines
//!                          checked against the snapshots → compare_live of the snapshots → LiveComparison
//! ```
//!
//! `bytes_identical` (the *identical* verdict) holds when both documents are unmodified, read from files of the same
//! size and CRC-32 (their load stamps) and have the same text, which is what a fresh load of the two files would find.
//! The encodings of the request are not used (a per-side encoding changes how bytes become text: the view reopens
//! instead, §1.11.13).

use std::sync::Arc;

use hikasa_core::{Pair, Side, Spec};
use hikasa_text::{CompareObserver, EditableDocument, LiveComparison, TextDocument, compare_live};

use super::compare::{LoadedPair, check_anchors, check_ignored};
use super::requests::TextCompareRequest;
use super::responses::TextCompareResponse;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// A relive's result (additive to the contract's [`Service::text_relive`]): the new live comparison and the response
/// a fresh comparison would give (the formats and their reasons for the File Info header, the warnings, the verdict).
#[derive(Debug)]
pub struct Relived {
    /// The comparison that follows the edits from now on.
    pub live: LiveComparison,
    /// The response of the comparison (its `comparison` is `live.current()`).
    pub response: TextCompareResponse,
}

impl Service {
    /// Checks the rules a text rule tab would write (§1.11.25): the patterns compile within the registry's regex limits,
    /// the closeness threshold is in 0–1,000 ‰, every element name is valid and not the reserved *Everything Else*
    /// (D-04). `invalid_input` with core's message; the warnings are those of the rules (none today). The typed form of
    /// what `text compare --rules-file` and `session edit --set` check.
    pub fn check_text_rules(
        &self,
        rules: &hikasa_text::TextRules,
    ) -> Result<Vec<hikasa_core::Warning>, ApiError> {
        rules
            .validate(self.registry().limits())
            .map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
        Ok(Vec::new())
    }

    /// Compares the live documents' snapshots again with `req`'s rules, per-side formats, anchors and ignored lines
    /// (§1.11.13; the documents are not re-read); the new [`LiveComparison`] replaces the view's. Run as a job
    /// (`JobKind::TextCompare`). Errors as `text_compare` (a rules file that does not parse, a format that does not
    /// exist, an anchor or ignored line beyond a document, a cancel).
    pub fn text_relive(
        &self,
        docs: Pair<&EditableDocument>,
        req: &TextCompareRequest,
        job: &JobControl,
        observer: Option<&mut dyn CompareObserver>,
    ) -> Result<LiveComparison, ApiError> {
        self.text_relive_full(docs, req, job, observer)
            .map(|r| r.live)
    }

    /// [`Service::text_relive`] with the response of the comparison (additive): what the view's header and status bar
    /// show after a relive.
    pub fn text_relive_full(
        &self,
        docs: Pair<&EditableDocument>,
        req: &TextCompareRequest,
        job: &JobControl,
        observer: Option<&mut dyn CompareObserver>,
    ) -> Result<Relived, ApiError> {
        run_guarded(|| {
            let pair = self.relive_pair(docs, req)?;
            let ctl = job.ctl();
            let live = compare_live(
                pair.compare_input(req),
                self.registry().limits(),
                Some(self.pool()),
                &ctl,
                observer,
            )
            .map_err(|e| ApiError::from_text(e, Side::Left, &pair.specs.left.canonical()))?;
            let response = pair.respond(req, Arc::clone(live.current()));
            Ok(Relived { live, response })
        })
    }

    /// The pair of a relive: the snapshots in place of the loaded documents, the rules and formats of `req`.
    fn relive_pair(
        &self,
        docs: Pair<&EditableDocument>,
        req: &TextCompareRequest,
    ) -> Result<LoadedPair, ApiError> {
        let mut warnings = Vec::new();
        let rules = self.text_rules_from(
            req.base_rules.as_ref(),
            req.rules_file.as_deref(),
            &req.rules_patch,
            &mut warnings,
        )?;
        // The specs name the documents (detection by file name); `-` and `clipboard://` are never re-read.
        let spec_of = |doc: &EditableDocument, raw: &std::ffi::OsStr| -> Spec {
            doc.location()
                .cloned()
                .unwrap_or_else(|| Spec::parse_os(raw, &self.spec_context()).unwrap_or(Spec::Stdin))
        };
        let specs = Pair::new(
            spec_of(docs.left, &req.left),
            spec_of(docs.right, &req.right),
        );
        let names = Pair::new(
            self.detection_name(&specs.left),
            self.detection_name(&specs.right),
        );
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let base = self.choose_format_with(
            req.format.as_deref(),
            names.as_ref().map(Option::as_deref),
            &overrides,
        )?;
        let (chosen, right_chosen) = self.per_side_formats(req, base)?;
        warnings.extend(chosen.warnings.iter().cloned());
        let snapshots: Pair<Arc<TextDocument>> =
            Pair::new(docs.left.snapshot(), docs.right.snapshot());
        check_anchors(&req.anchors, &snapshots.left, &snapshots.right)?;
        let ignored = check_ignored(
            req,
            Pair::new(&*snapshots.left, &*snapshots.right),
            Pair::new(&specs.left, &specs.right),
            &mut warnings,
        )?;
        let bytes_identical = same_files(docs);
        Ok(LoadedPair {
            specs,
            docs: snapshots,
            chosen,
            right_chosen,
            ignored,
            rules,
            bytes_identical,
            warnings,
        })
    }
}

/// True when both documents hold, unmodified, files of the same size and CRC-32 with the same text.
fn same_files(docs: Pair<&EditableDocument>) -> bool {
    let stamp = |d: &EditableDocument| d.splice_source().map(|s| s.stamp);
    match (stamp(docs.left), stamp(docs.right)) {
        (Some(a), Some(b)) => {
            !docs.left.is_modified()
                && !docs.right.is_modified()
                && a.size == b.size
                && a.crc32 == b.crc32
                && docs.left.document().rope() == docs.right.document().rope()
        }
        _ => false,
    }
}
