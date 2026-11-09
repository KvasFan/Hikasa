//! Saving edited documents (TC-001, TC-003, TC-005, CLI-003; D-272…D-277, contract §1.10.12–§1.10.14).
//!
//! ```text
//! apply_save_options(&mut doc, opts)     the format's *when saving* options on edited and new lines: one undo step
//! plan_save(vfs, [(side, &doc, &req)])   a job (JobKind::TextSave): per document the target (in place, Save As,
//!                                        --savetarget; links followed), its stat, the splice source read once
//!                                        (SourceMap, CRC), the SaveContent, the backup, the questions
//!   → SavePlan { plan: ops::Plan of `write` items, items, exec, warnings }     serializable; dry-run lines
//! SaveConfirmation::new(&plan, policy, mode) … finish(plan)                 W6's machine; driven through `Ask`
//! SaveSource::new(vfs, &plan, docs)       a read-only one-file-per-item Vfs: open_read(target) streams the pieces
//! execute_save(vfs, plan, source, ctl)    hikasa_vfs::ops writes (Action::Write); per-item results
//!   → SaveReport { op, written, saved }   EditableDocument::saved_to(saved[i]) makes the file the next splice source
//! ```
//!
//! **`hikasa_vfs::ops` stays the only writer** (§1.6.2): the plan is an `ops::Plan` of kind `Batch` whose items use
//! `Action::Write`; this module reads through `&dyn Vfs` only (§1.10.25 check 2).

mod confirm;
mod content;
mod convert;
mod plan;
mod source;

use std::sync::Arc;

use hikasa_core::{Side, Spec, Warning};
use hikasa_formats::{CompiledFormat, TextEncoding};
use hikasa_vfs::ops::{BackupPolicy, ExecOptions, ItemStatus, OpReport, Plan};
use serde::{Deserialize, Serialize};

use crate::document::{EolStyle, EolSummary, LoadStamp};
use crate::edit::{
    EditError, EditOutcome, EditRequest, EditableDocument, GroupHint, LineMark, SavedFile,
};
use crate::transform;

pub use confirm::{SaveAsk, SaveConfirmation, SavePolicy};
pub use content::{
    ContentIssue, ContentOptions, Piece, ReencodeReason, SaveContent, ScanError, SourceMap,
    UNMAPPABLE_LISTED, UnmappableChar, save_content,
};
pub use convert::{
    ConvertCounts, ConvertPieces, ConvertSpec, ConvertStream, convert_counts, converted_content,
};
pub use plan::{SaveEnv, execute_save, plan_save, plan_save_with};
pub use source::SaveSource;

/// What a save does with characters the document's encoding cannot write (§1.10.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmappablePolicy {
    /// The question `Unmappable` (D-274's open default).
    #[default]
    Ask,
    /// The item is not written (an error naming the first characters).
    Refuse,
    /// They are written as `?` (warning `unmappable_replaced`); never as numeric references.
    Replace,
}

impl UnmappablePolicy {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            UnmappablePolicy::Ask => "ask",
            UnmappablePolicy::Refuse => "refuse",
            UnmappablePolicy::Replace => "replace",
        }
    }
}

/// One document's save (§1.10.13). The encoding, the BOM and the terminators are the document's own
/// ([`EditRequest::SetEncoding`], [`EditRequest::SetEols`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveRequest {
    /// Save As to this file; `None`: Save (the view's `--savetarget`, else in place, else an item error asking for a
    /// target).
    pub target: Option<Spec>,
    /// The backup of a replaced file (`BackupPolicy::target(…, rel = None)`); `None`: no backup.
    pub backup: Option<BackupPolicy>,
    /// What unmappable characters do.
    pub unmappable: UnmappablePolicy,
    /// The format's External conversion with a `save` command line (FF-030, FF-031, D-39), from
    /// [`CompiledFormat::conversion`]: the encoded document is `%s`, the program writes `%t`, and `%t`'s bytes are
    /// written. Additive (M2 W10b): the document does not know its format.
    pub conversion: Option<hikasa_formats::ExternalConversion>,
    /// The conversions of `hikasa text convert` (W10b note 60): applied line by line while the content is written,
    /// the document itself never edited. Additive.
    pub transform: Option<ConvertSpec>,
}

impl SaveRequest {
    /// An in-place save with these options.
    pub fn in_place(backup: Option<BackupPolicy>, unmappable: UnmappablePolicy) -> SaveRequest {
        SaveRequest {
            target: None,
            backup,
            unmappable,
            conversion: None,
            transform: None,
        }
    }

    /// The External conversion of `format`, when it has a `save` command line.
    pub fn conversion_of(format: &CompiledFormat) -> Option<hikasa_formats::ExternalConversion> {
        match format.conversion() {
            hikasa_formats::Conversion::External(conv) if conv.save.is_some() => Some(conv.clone()),
            _ => None,
        }
    }
}

/// Where an item writes, as the request asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveMode {
    /// The document's own file.
    InPlace,
    /// Save As: the location moves to the target after success.
    SaveAs,
    /// The view's `--savetarget`: the location stays, the target becomes the splice source.
    SaveTarget,
}

impl SaveMode {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            SaveMode::InPlace => "in_place",
            SaveMode::SaveAs => "save_as",
            SaveMode::SaveTarget => "save_target",
        }
    }
}

/// What an item writes, without the pieces (the plan's and the dry run's view of a [`SaveContent`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveContentSummary {
    /// Bytes written.
    pub bytes: u64,
    /// The encoding.
    pub encoding: TextEncoding,
    /// A BOM is written.
    pub bom: bool,
    /// The terminators written.
    pub eols: EolSummary,
    /// Bytes spliced from the source.
    pub spliced_bytes: u64,
    /// Physical lines encoded.
    pub encoded_lines: u32,
    /// Lines the format's *when saving* options changed before the plan ([`apply_save_options`]).
    pub pre_save: u64,
}

impl SaveContentSummary {
    /// The summary of `content`.
    pub fn of(content: &SaveContent, pre_save: u64) -> SaveContentSummary {
        SaveContentSummary {
            bytes: content.bytes,
            encoding: content.encoding,
            bom: content.bom,
            eols: content.eols,
            spliced_bytes: content.spliced_bytes,
            encoded_lines: content.encoded_lines,
            pre_save,
        }
    }
}

/// The class of a save question (§1.10.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveQuestion {
    /// The target's content differs from the splice source: Yes writes a whole re-encode.
    ChangedOnDisk,
    /// A DOS `R` or `Vfs::writable` false on the existing target: Yes clears `R` or writes through the folder.
    ReadOnlyTarget,
    /// Characters the encoding cannot write: Yes writes them as `?`.
    Unmappable,
    /// Edited lines held U+FFFD of a lossy decode: Yes writes them.
    LossyRewritten,
    /// Save As onto an existing file that is not the document's own: Yes replaces it (with its backup).
    ReplaceExisting,
    /// The backup would replace a file this document's saves did not make.
    OverwriteBackup,
}

impl SaveQuestion {
    /// The classes in the order they are asked.
    pub const ALL: [SaveQuestion; 6] = [
        SaveQuestion::ChangedOnDisk,
        SaveQuestion::ReadOnlyTarget,
        SaveQuestion::Unmappable,
        SaveQuestion::LossyRewritten,
        SaveQuestion::ReplaceExisting,
        SaveQuestion::OverwriteBackup,
    ];

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            SaveQuestion::ChangedOnDisk => "changed_on_disk",
            SaveQuestion::ReadOnlyTarget => "read_only_target",
            SaveQuestion::Unmappable => "unmappable",
            SaveQuestion::LossyRewritten => "lossy_rewritten",
            SaveQuestion::ReplaceExisting => "replace_existing",
            SaveQuestion::OverwriteBackup => "overwrite_backup",
        }
    }
}

/// What the plan found about an item (§1.10.14): the questions, and the facts a dry run shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SaveIssue {
    /// The target changed on disk since it was loaded or saved (a question).
    ChangedOnDisk,
    /// The target is read-only (a question).
    ReadOnlyTarget,
    /// Unmappable characters (a question under [`UnmappablePolicy::Ask`]).
    Unmappable {
        /// How many.
        count: u64,
        /// The first ones.
        first: Vec<UnmappableChar>,
    },
    /// Rewritten lossy lines (a question).
    LossyRewritten {
        /// Physical lines.
        lines: u64,
    },
    /// Save As over an existing file (a question).
    ReplaceExisting,
    /// The backup replaces a file (a question).
    OverwriteBackup {
        /// The backup.
        backup: Spec,
    },
    /// The whole text is encoded (no question; warning `save_reencoded`).
    Reencoded {
        /// Why.
        reason: ReencodeReason,
    },
    /// The file was touched (a new time) but its content is the one loaded (no question; warning `source_touched`).
    SourceTouched,
}

impl SaveIssue {
    /// The question the issue asks, if any (`Unmappable` asks only under [`UnmappablePolicy::Ask`], which the plan
    /// records by keeping the issue).
    pub fn question(&self) -> Option<SaveQuestion> {
        match self {
            SaveIssue::ChangedOnDisk => Some(SaveQuestion::ChangedOnDisk),
            SaveIssue::ReadOnlyTarget => Some(SaveQuestion::ReadOnlyTarget),
            SaveIssue::Unmappable { .. } => Some(SaveQuestion::Unmappable),
            SaveIssue::LossyRewritten { .. } => Some(SaveQuestion::LossyRewritten),
            SaveIssue::ReplaceExisting => Some(SaveQuestion::ReplaceExisting),
            SaveIssue::OverwriteBackup { .. } => Some(SaveQuestion::OverwriteBackup),
            SaveIssue::Reencoded { .. } | SaveIssue::SourceTouched => None,
        }
    }
}

/// One item of a save plan: one document, one file (`SavePlan::plan.items` has the same index).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveItem {
    /// The pane (`None`: a Text Edit view, a file of `text convert`).
    pub side: Option<Side>,
    /// The file written: the file a link leads to for a save through a link.
    pub target: Spec,
    /// The link the save was asked through (it stays a link).
    pub via_link: Option<Spec>,
    /// The target does not exist: the save creates it.
    pub creates: bool,
    /// Where the save writes, as asked.
    pub mode: SaveMode,
    /// What is written.
    pub content: SaveContentSummary,
    /// The questions and facts.
    pub issues: Vec<SaveIssue>,
    /// The save program of an External conversion (FF-030), when one ran.
    pub external: Option<String>,
    /// The document generation the content was made from: an execution with another refuses the item.
    pub generation: u64,
}

impl SaveItem {
    /// The questions of the item, in the order they are asked.
    pub fn questions(&self) -> Vec<SaveQuestion> {
        let mut questions: Vec<SaveQuestion> =
            self.issues.iter().filter_map(SaveIssue::question).collect();
        questions.sort();
        questions.dedup();
        questions
    }
}

/// A save plan (§1.10.13): an `ops::Plan` of kind `Batch` whose items are `write` items (or skips and planning
/// errors), one per document, and what each writes. Serializable (`hikasa.text_convert.v1`'s and
/// `hikasa.text_find.v1`'s `plan`, W11's confirmation list); the pieces of the content are kept beside it for
/// [`SaveSource::new`] and are not serialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavePlan {
    /// What `hikasa_vfs::ops` executes.
    pub plan: Plan,
    /// One per document, the plan's items' order.
    pub items: Vec<SaveItem>,
    /// Archive backups (D-168).
    pub exec: ExecOptions,
    /// `source_touched`, `save_reencoded`, `unmappable_replaced`, `lossy_line_rewritten`.
    pub warnings: Vec<Warning>,
    /// The questions were answered ([`SaveConfirmation::finish`]) or there were none: [`execute_save`] refuses the
    /// items of an unconfirmed plan that ask.
    pub confirmed: bool,
    /// The content of each item and the source it splices from (not serialized).
    #[serde(skip)]
    pub(crate) contents: Vec<Option<ItemContent>>,
}

/// What [`SaveSource`] streams for one item.
#[derive(Debug, Clone)]
pub(crate) struct ItemContent {
    pub(crate) content: Arc<SaveContent>,
    /// The splice source the `Source` pieces read, with the stamp they are checked against.
    pub(crate) source: Option<crate::edit::SpliceSource>,
}

impl PartialEq for SavePlan {
    /// The serialized parts.
    fn eq(&self, other: &SavePlan) -> bool {
        self.plan == other.plan
            && self.items == other.items
            && self.exec == other.exec
            && self.warnings == other.warnings
            && self.confirmed == other.confirmed
    }
}

impl SavePlan {
    /// The questions left (0 once confirmed).
    pub fn questions(&self) -> u64 {
        if self.confirmed {
            return 0;
        }
        self.runs()
            .map(|(_, item)| item.questions().len() as u64)
            .sum()
    }

    /// The items that will run (no planning error, not skipped), with their index.
    pub(crate) fn runs(&self) -> impl Iterator<Item = (usize, &SaveItem)> {
        self.items.iter().enumerate().filter(|(i, _)| {
            self.plan
                .items
                .get(*i)
                .is_some_and(|item| item.error.is_none() && item.skip.is_none())
        })
    }

    /// The dry run's human lines (§1.10.13): `write <target> (<bytes> bytes, <encoding>[ BOM], <eol style>[, backup
    /// <spec>][, via <link>])` per item that writes; `skip <target> (<reason>)` and `fail <target>: <message>` for the
    /// others.
    pub fn human_lines(&self) -> Vec<String> {
        self.items
            .iter()
            .zip(&self.plan.items)
            .map(|(item, planned)| {
                let target = item.target.canonical();
                if let Some(error) = &planned.error {
                    return format!("fail {target}: {}", error.message);
                }
                if let Some(skip) = planned.skip {
                    return format!("skip {target} ({})", skip.as_str());
                }
                let c = &item.content;
                let mut line = format!(
                    "write {target} ({} bytes, {}{}, {}",
                    c.bytes,
                    c.encoding.name(),
                    if c.bom { " BOM" } else { "" },
                    eol_words(c.eols.style())
                );
                let backup = planned.backup.clone().or_else(|| {
                    planned
                        .target
                        .outermost_file()
                        .and_then(|outer| self.exec.archive_backups.get(&outer.canonical()))
                        .cloned()
                });
                if let Some(backup) = backup {
                    line.push_str(&format!(", backup {}", backup.canonical()));
                }
                if let Some(link) = &item.via_link {
                    line.push_str(&format!(", via {}", link.canonical()));
                }
                line.push(')');
                line
            })
            .collect()
    }
}

/// The words of a terminator style in a dry-run line.
fn eol_words(style: EolStyle) -> &'static str {
    match style {
        EolStyle::Lf => "LF",
        EolStyle::CrLf => "CRLF",
        EolStyle::Cr => "CR",
        EolStyle::Mixed => "mixed line endings",
        EolStyle::None => "no line endings",
    }
}

/// The outcome of [`execute_save`] (§1.10.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// Per-item results, W3's shape (the plan's items' order).
    pub op: OpReport,
    /// The new stamp of each written file (`None` for an item not done).
    pub written: Vec<Option<LoadStamp>>,
    /// What each done item wrote, for [`EditableDocument::saved_to`] of its document.
    pub saved: Vec<Option<SavedFile>>,
}

impl SaveReport {
    /// True when every item was done.
    pub fn ok(&self) -> bool {
        self.op.ok()
    }

    /// The status of item `i`.
    pub fn status(&self, i: usize) -> Option<ItemStatus> {
        self.op.results.get(i).map(|result| result.status)
    }
}

/// The format's *when saving* options (docs/10 §1; §1.10.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveOptions {
    /// `save_trim_trailing_ws`.
    pub trim_trailing_ws: bool,
    /// `save_leading_spaces_to_tabs`.
    pub leading_spaces_to_tabs: bool,
}

impl SaveOptions {
    /// The options of `format`.
    pub fn of(format: &CompiledFormat) -> SaveOptions {
        let text = format.text();
        SaveOptions {
            trim_trailing_ws: text.save_trim_trailing_ws,
            leading_spaces_to_tabs: text.save_leading_spaces_to_tabs,
        }
    }

    /// True when no option is on.
    pub fn is_empty(&self) -> bool {
        !self.trim_trailing_ws && !self.leading_spaces_to_tabs
    }
}

/// What [`apply_save_options`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreSave {
    /// The edit (one undo step, "Save options"): a live comparison's `note_edit` takes it.
    pub outcome: EditOutcome,
    /// Physical lines changed.
    pub lines: u64,
}

/// Applies the format's *when saving* options to the **edited and new** lines of `doc` (§1.10.12, D-272's open
/// default): every physical line with a piece marked [`LineMark::Edited`] gets Trim Trailing Whitespace, then Leading
/// Spaces to Tabs (the TC-063 rules of [`crate::transform`](mod@crate::transform), tab size from the document's
/// options), all as **one** edit (one undo step) applied to the live document, so what is shown is what is written;
/// unchanged lines stay byte-exact. `Ok(None)` when nothing changes. The next [`plan_save`] of the document counts the lines
/// ([`SaveContentSummary::pre_save`]). Pure and in memory (the adapter calls it before planning).
pub fn apply_save_options(
    doc: &mut EditableDocument,
    opts: SaveOptions,
) -> Result<Option<PreSave>, EditError> {
    if opts.is_empty() {
        return Ok(None);
    }
    let ts = u32::from(doc.options().tab_size.max(1));
    let rule = |line: &str| -> Option<String> {
        let mut out: Option<String> = None;
        if opts.trim_trailing_ws {
            out = transform::trim_trailing(line);
        }
        if opts.leading_spaces_to_tabs {
            let current = out.as_deref().unwrap_or(line);
            if let Some(tabbed) = transform::leading_spaces_to_tabs(current, ts) {
                out = Some(tabbed);
            }
        }
        out
    };
    let n = doc.line_count();
    let mut edits = Vec::new();
    let mut lines = 0u64;
    let mut p = 0u32;
    while p < n {
        // A run of consecutive edited physical lines.
        let end = doc.phys_end(p);
        if !(p..end).any(|l| doc.mark(crate::LineIdx(l)) == LineMark::Edited) {
            p = end;
            continue;
        }
        let start = p;
        let mut stop = end;
        while stop < n {
            let next = doc.phys_end(stop);
            if !(stop..next).any(|l| doc.mark(crate::LineIdx(l)) == LineMark::Edited) {
                break;
            }
            stop = next;
        }
        let out = transform::content(doc, start..stop, rule);
        if let Some(EditRequest::Batch { edits: run, .. }) = out.request {
            edits.extend(run);
            lines += out.lines;
        }
        p = stop;
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let request = EditRequest::Batch {
        edits,
        keep_eols: true,
    };
    let outcome = doc.apply(&request, GroupHint::None)?;
    doc.note_pre_save(lines);
    Ok(Some(PreSave { outcome, lines }))
}
