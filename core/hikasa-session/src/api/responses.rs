//! Service responses (§1.8.4): [`QuickResponse`], [`TextCompareResponse`], [`FormatChoice`],
//! [`PreparedFolderCompare`], [`FolderCompareResponse`].

use std::collections::BTreeMap;
use std::sync::Arc;

use hikasa_core::{CompareVerdict, ContentOutcome, ContentResult, ExitCode, Pair, Spec, Warning};
use hikasa_folder::{DisplayOptions, FolderComparison, FolderModel, FolderRules};
use hikasa_formats::{DetectReason, EnabledOverrides, FormatId, FormatKind};
use hikasa_report::json::{
    DetectReasonJson, FolderCompareJson, FormatJson, TextCompareJson, folder_compare_json,
    text_compare_json,
};
use hikasa_text::{TextComparison, TextRules, ViewOptions};

use super::requests::QuickMethod;

/// The effective format of a file comparison (D-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatChoice {
    /// Id (`None` = Everything Else).
    pub id: Option<FormatId>,
    /// Display name.
    pub name: String,
    /// Kind.
    pub kind: FormatKind,
    /// Why it was chosen.
    pub reason: DetectReason,
}

impl FormatChoice {
    /// The JSON shape `{"id", "name", "kind", "reason"}` (§4.3.1, §4.3.2). Additive.
    pub fn json(&self) -> FormatJson {
        FormatJson {
            id: self.id.as_ref().map(|id| id.as_str().to_owned()),
            name: self.name.clone(),
            kind: self.kind.as_str(),
            reason: Some(DetectReasonJson::new(&self.reason)),
        }
    }
}

/// Result of `quick`.
#[derive(Debug, Clone)]
pub struct QuickResponse {
    /// Left spec.
    pub left: Spec,
    /// Right spec.
    pub right: Spec,
    /// Method used.
    pub method: QuickMethod,
    /// Effective format (`None` for the size, CRC and binary methods).
    pub format: Option<FormatChoice>,
    /// Content outcome (its `warnings` are the comparison's warnings, e.g. `format_mismatch`).
    pub outcome: ContentOutcome,
    /// The request's ignore-unimportant (exit code and `hikasa.quick.v1`). Added in the wave-4 review.
    pub ignore_unimportant: bool,
    /// Exit code (§2.21).
    pub exit_code: ExitCode,
}

/// Result of `text compare`.
#[derive(Debug, Clone)]
pub struct TextCompareResponse {
    /// The comparison (shared with GUI views and reports).
    pub comparison: Arc<TextComparison>,
    /// Effective format (D-15; the left side's when the right one has its own, D-281).
    pub format: FormatChoice,
    /// The right side's own format when the sides' formats differ (`--left-format`, `--right-format`, the
    /// session's `left_format`/`right_format`; FF-003, D-281). Additive (M2 W10b).
    pub right_format: Option<FormatChoice>,
    /// The checks of the ignored lines each side has (`hikasa_text::ignored_check`; `None` without marks): what
    /// a session saves as `ignored_check_left`/`ignored_check_right` (TC-024, §1.10.17). Additive (M2 W10b).
    pub ignored_check: Pair<Option<u32>>,
    /// Detection per side by name (`None` for an unnamed side); informational (`format_mismatch`).
    pub detection: Pair<Option<DetectReason>>,
    /// Verdict under the request's `ignore_unimportant`.
    pub verdict: ContentResult,
    /// The request's ignore-unimportant, which the verdict, the exit code, the JSON and the reports use.
    /// Added in the wave-4 review.
    pub ignore_unimportant: bool,
    /// Exit code (§2.21).
    pub exit_code: ExitCode,
    /// Session warnings (rules-file keys, `format_mismatch`) followed by the comparison's warnings.
    pub warnings: Vec<Warning>,
    /// The sides with unsaved edits (a Text Compare view's report, [`Service::text_report_source`]; M2 W13c,
    /// D-366): the report header marks them `<spec> (unsaved edits)`. Additive; false on both sides for every
    /// comparison of files.
    ///
    /// [`Service::text_report_source`]: crate::Service::text_report_source
    pub unsaved: Pair<bool>,
}

impl TextCompareResponse {
    /// The `hikasa.text_compare.v1` result for `view` (§4.3.2), with the format detection reasons that only the
    /// session knows (`format.reason`, `detection`). Additive: the CLI and the GUI's "Report…" use it.
    ///
    /// `view.ignore_unimportant` is replaced by the response's own flag, so the JSON verdict, statistics and
    /// items always agree with `exit_code` (wave-4 review).
    pub fn json(&self, view: &ViewOptions) -> TextCompareJson {
        let view = ViewOptions {
            ignore_unimportant: self.ignore_unimportant,
            ..view.clone()
        };
        let mut json = text_compare_json(&self.comparison, &view).with_detection(
            Some(&self.format.reason),
            self.detection.left.as_ref(),
            self.detection.right.as_ref(),
        );
        // The right side's own format (D-281): why it was chosen.
        if let (Some(format), Some(choice)) = (json.right_format.as_mut(), &self.right_format) {
            format.reason = Some(DetectReasonJson::new(&choice.reason));
        }
        json
    }
}

/// A prepared folder comparison (`Service::folder_prepare`).
#[derive(Clone)]
pub struct PreparedFolderCompare {
    pub(crate) left: Spec,
    pub(crate) right: Spec,
    pub(crate) rules: FolderRules,
    pub(crate) text_rules: Option<TextRules>,
    pub(crate) overrides: EnabledOverrides,
    pub(crate) ignore_unimportant: bool,
    pub(crate) model: Arc<FolderModel>,
    pub(crate) warnings: Vec<Warning>,
}

impl PreparedFolderCompare {
    /// The live model (empty until `folder_run` starts filling it).
    pub fn model(&self) -> &Arc<FolderModel> {
        &self.model
    }

    /// The formats this comparison turns on or off (FF-002): what a child view of a row opens with.
    pub fn format_overrides(&self) -> &BTreeMap<String, bool> {
        &self.rules.format_overrides
    }

    /// Effective folder rules.
    pub fn rules(&self) -> &FolderRules {
        &self.rules
    }

    /// The base specs (additive).
    pub fn specs(&self) -> Pair<&Spec> {
        Pair::new(&self.left, &self.right)
    }

    /// The text rules of the injected `RulesComparator` (`Some` only for content method Rules; additive).
    pub fn text_rules(&self) -> Option<&TextRules> {
        self.text_rules.as_ref()
    }

    /// Ignore-unimportant of the verdict: the request's flag or the criteria word `ignore-unimportant`
    /// (additive).
    pub fn ignore_unimportant(&self) -> bool {
        self.ignore_unimportant
    }

    /// Warnings of the preparation (unknown keys of the rules files; additive).
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The verdict a folder view shows and reports when it closes (§1.8.7 items 22 and 37; additive):
    /// `Incomplete` while `scanning` (`folder_run` has not returned) and after a scan that did not finish
    /// (stopped: unscanned folders are unknown, D-02); otherwise the model's verdict under `ignore_unimportant`.
    pub fn view_verdict(&self, ignore_unimportant: bool, scanning: bool) -> CompareVerdict {
        let read = self.model.read();
        if scanning || !read.is_complete() {
            CompareVerdict::Incomplete
        } else {
            read.verdict(ignore_unimportant)
        }
    }
}

impl std::fmt::Debug for PreparedFolderCompare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedFolderCompare")
            .field("left", &self.left.canonical())
            .field("right", &self.right.canonical())
            .field("ignore_unimportant", &self.ignore_unimportant)
            .finish_non_exhaustive()
    }
}

/// Result of `folder compare`.
#[derive(Debug, Clone)]
pub struct FolderCompareResponse {
    /// The finished (or cancelled, incomplete) comparison.
    pub comparison: FolderComparison,
    /// Verdict under the effective ignore-unimportant flag (D-02).
    pub verdict: CompareVerdict,
    /// The effective ignore-unimportant: the request's flag or the criteria word `ignore-unimportant`
    /// (§2.6.6 row 4). The verdict, the exit code, the JSON and the reports use it. Added in the wave-4 review.
    pub ignore_unimportant: bool,
    /// Exit code (§2.21).
    pub exit_code: ExitCode,
    /// Preparation warnings followed by the comparison's warnings.
    pub warnings: Vec<Warning>,
}

impl FolderCompareResponse {
    /// The `hikasa.folder_compare.v1` result for `display` (§4.3.3). Additive.
    ///
    /// `display.ignore_unimportant` is replaced by the effective flag ([`FolderCompareResponse::ignore_unimportant`]),
    /// so `result.verdict`, the statistics and the categories always agree with `exit_code` (decision 4 of the
    /// wave-4 review).
    pub fn json(&self, display: &DisplayOptions) -> FolderCompareJson {
        let display = DisplayOptions {
            ignore_unimportant: self.ignore_unimportant,
            ..*display
        };
        folder_compare_json(&self.comparison, &display)
    }
}
