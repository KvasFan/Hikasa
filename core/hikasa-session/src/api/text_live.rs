//! Live text comparisons (M2 W10a; contract §1.10.7, §1.10.10, §1.10.23): [`Service::text_compare_live`] opens
//! a comparison the views edit, with one [`EditableDocument`] per pane and the [`Editability`] of each.
//!
//! ```text
//! text_compare_live(req, opts) = load_text_pair (as text compare)
//!                                → compare_live (the comparison and what an exact recompare needs)
//!                                → per side: EditFacts (the open, Vfs::writable, the format, the document, opts)
//!                                            → Editability → EditableDocument::new(doc, EditOptions, editability)
//! ```
//!
//! [`EditFacts::editability`] maps §1.10.7's table, every cause in [`EditBlock`]'s order:
//!
//! | Fact | Block |
//! |---|---|
//! | the side could not be loaded (the view's, W11) | `LoadFailed` |
//! | the comparison was cancelled (the view's; a later run clears it) | `Cancelled` |
//! | `Vfs::writable` is `Some(false)` for a file | `ReadOnlyLocation` |
//! | `Vfs::writable` is `Some(false)` for an archive member | `ArchiveReadOnly` |
//! | the format is *Sorted*, or *External* without `save` | `Converted` |
//! | `TextDocument::truncated` | `Truncated` |
//! | [`LiveOptions::parent_read_only`] | `ParentReadOnly` |
//! | [`LiveOptions::session_read_only`] | `SessionReadOnly` |
//! | [`LiveOptions::read_only`] (`--ro`, `--ro1`, `--ro2`) | `CommandLine` |
//! | the binary verdict | `Binary` |
//!
//! `Vfs::writable` = `None` (the back-end cannot tell) is writable: the save finds out. A document without a
//! location (`-`, `clipboard://`) is editable with `can_save_in_place = false` (Save becomes Save As). The calls
//! that write a pane check [`super::ensure_editable`] first.

use std::sync::Arc;

use hikasa_core::{Pair, Side, Spec};
use hikasa_folder::ops::ReadOnlySides;
use hikasa_formats::{CompiledFormat, Conversion};
use hikasa_text::{
    CompareObserver, EditBlock, EditOptions, Editability, EditableDocument, LiveComparison,
    LoadStamp, TextDocument, compare_live,
};
use hikasa_vfs::Vfs;

use super::requests::TextCompareRequest;
use super::responses::TextCompareResponse;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};
use crate::settings::options::Options;

/// How a live comparison is opened (§1.10.22, §1.10.23): what the view's switches, its parent and its session say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveOptions {
    /// `--ro`, `--ro1`, `--ro2` (`OpenOptions::read_only`, [`crate::ipc::ReadOnly::sides`]): `CommandLine`.
    pub read_only: ReadOnlySides,
    /// The read-only sides of the folder comparison the view was opened from (D-184's *child views carry the
    /// flag*): `ParentReadOnly`.
    pub parent_read_only: ReadOnlySides,
    /// The session's `[specs] read_only_left` / `read_only_right` ([`Service::session_read_only`]):
    /// `SessionReadOnly`. Additive to the contract's fields (W10a).
    pub session_read_only: ReadOnlySides,
    /// `--savetarget=<file>`: an absolute canonical spec the saves write instead of the right side (§1.10.14,
    /// D-275's open default): `text_compare_live` sets it as the right document's save target.
    pub save_target: Option<String>,
    /// `--nobackups`: no backup on any save of this view ([`Service::live_save_request`]).
    pub no_backups: bool,
}

/// An open live comparison (§1.10.23): the response of the first comparison, the live comparison, one
/// editable document per pane, the load stamps of the saves (W10b).
#[derive(Debug)]
pub struct LiveTextView {
    /// The first comparison's response; its `comparison` is `live.current()` at the open.
    pub response: TextCompareResponse,
    /// The comparison that follows the edits.
    pub live: LiveComparison,
    /// The panes' documents, each with its editability.
    pub docs: Pair<EditableDocument>,
    /// The resolved specs of the panes (additive: what a save writes and the File Info header names).
    pub specs: Pair<Spec>,
    /// The options the view was opened with (additive: the save target and `--nobackups` for W10b's saves).
    pub options: LiveOptions,
    /// The load stamp of each pane (§1.10.13: size, time, CRC-32 of the bytes read; `None` for standard input and
    /// the clipboard), which the documents also carry as their splice source (W10b).
    pub stamps: Pair<Option<LoadStamp>>,
}

/// What decides whether one pane may be edited (§1.10.7): [`EditFacts::editability`] maps it, so the view
/// computes the blocks it finds itself (`LoadFailed`, `Cancelled`) with the same rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditFacts {
    /// The side could not be loaded.
    pub load_failed: bool,
    /// The comparison was cancelled.
    pub cancelled: bool,
    /// The document has a location to save to (false for `-`, `clipboard://` and text pairs).
    pub has_location: bool,
    /// The location is a member of an archive.
    pub archive_member: bool,
    /// `Vfs::writable` of the location (`None`: cannot tell, or no location).
    pub writable: Option<bool>,
    /// The format converts without a save path (*Sorted*, *External* without `save`).
    pub converted: bool,
    /// The document was cut at the D-43 limits.
    pub truncated: bool,
    /// The parent folder comparison's side is read-only.
    pub parent_read_only: bool,
    /// The session's side is read-only.
    pub session_read_only: bool,
    /// `--ro`, `--ro1`, `--ro2`.
    pub command_line: bool,
    /// The content is binary.
    pub binary: bool,
}

impl EditFacts {
    /// §1.10.7's table: every cause, in [`EditBlock`]'s order; `can_save_in_place` when the document has a location.
    pub fn editability(&self) -> Editability {
        let location = if self.writable == Some(false) {
            Some(if self.archive_member {
                EditBlock::ArchiveReadOnly
            } else {
                EditBlock::ReadOnlyLocation
            })
        } else {
            None
        };
        let blocks = [
            (self.load_failed, EditBlock::LoadFailed),
            (self.cancelled, EditBlock::Cancelled),
            (self.converted, EditBlock::Converted),
            (self.truncated, EditBlock::Truncated),
            (self.parent_read_only, EditBlock::ParentReadOnly),
            (self.session_read_only, EditBlock::SessionReadOnly),
            (self.command_line, EditBlock::CommandLine),
            (self.binary, EditBlock::Binary),
        ]
        .into_iter()
        .filter_map(|(on, block)| on.then_some(block))
        .chain(location);
        Editability::with_blocks(blocks, self.has_location)
    }
}

/// True for a format whose conversion has no way back (FF-031): *Sorted*, and *External* without a `save` command.
pub fn converts_without_save(format: &CompiledFormat) -> bool {
    match format.conversion() {
        Conversion::None => false,
        Conversion::Sorted => true,
        Conversion::External(external) => external.save.is_none(),
    }
}

/// The editing options of a pane (§1.10.3): the format's tab size, tab-as-spaces, word characters and
/// characters-per-line limit; `[text_editing] auto_indent`, `backspace_unindents` and `default_line_ending`
/// (resolved for `platform`, D-290).
pub fn edit_options(
    format: &CompiledFormat,
    options: &Options,
    platform: crate::host::Platform,
) -> EditOptions {
    let text = format.text();
    EditOptions {
        tab_size: text.tab_size,
        tab_as_spaces: text.tab_as_spaces,
        word_chars: text.word_chars.clone(),
        auto_indent: options.text_editing.auto_indent,
        backspace_unindents: options.text_editing.backspace_unindents,
        default_eol: options.text_editing.default_line_ending.resolve(platform),
        chars_per_line_limit: text.chars_per_line_limit,
    }
}

impl Service {
    /// Opens a live comparison (§1.10.23, W10a): the steps of [`Service::text_compare`] (the request's anchors
    /// included), but `compare_live`, which keeps what an exact recompare needs; then one [`EditableDocument`] per
    /// pane with its [`Editability`] (module documentation) and the editing options of the format and the
    /// settings ([`edit_options`]). Errors as `text_compare`.
    pub fn text_compare_live(
        &self,
        req: &TextCompareRequest,
        opts: LiveOptions,
        job: &JobControl,
        observer: Option<&mut dyn CompareObserver>,
    ) -> Result<LiveTextView, ApiError> {
        run_guarded(|| self.text_compare_live_inner(None, req, opts, job, observer))
    }

    /// [`Service::text_compare_live`] on the back-end `vfs` (additive, as `report_plan_on`): a test's `MemoryVfs`
    /// with its read-only flags and faults. Standard input and the clipboard are not on it.
    pub fn text_compare_live_on(
        &self,
        vfs: &dyn Vfs,
        req: &TextCompareRequest,
        opts: LiveOptions,
        job: &JobControl,
        observer: Option<&mut dyn CompareObserver>,
    ) -> Result<LiveTextView, ApiError> {
        run_guarded(|| self.text_compare_live_inner(Some(vfs), req, opts, job, observer))
    }

    fn text_compare_live_inner(
        &self,
        on: Option<&dyn Vfs>,
        req: &TextCompareRequest,
        opts: LiveOptions,
        job: &JobControl,
        mut observer: Option<&mut dyn CompareObserver>,
    ) -> Result<LiveTextView, ApiError> {
        let loaded = self.load_text_pair(on, req, job, &mut observer)?;
        let ctl = job.ctl();
        let live = compare_live(
            loaded.compare_input(req),
            self.registry().limits(),
            Some(self.pool()),
            &ctl,
            observer,
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, &loaded.specs.left.canonical()))?;
        let router = self.router();
        let vfs: &dyn Vfs = on.unwrap_or(&*router);
        // Each pane edits with its own format's options (a format per pane, D-281).
        let left_format = Arc::clone(&loaded.chosen.format);
        let right_format = loaded
            .right_chosen
            .as_ref()
            .map_or_else(|| Arc::clone(&left_format), |c| Arc::clone(&c.format));
        let fresh = self.fresh_options();
        let specs = loaded.specs.clone();
        let docs = loaded.docs.clone();
        let editable = |side: Side, spec: &Spec, doc: &Arc<TextDocument>| {
            let format = if side == Side::Right {
                &right_format
            } else {
                &left_format
            };
            let edit = edit_options(format, &fresh, self.env().platform);
            let facts = edit_facts(vfs, side, spec, doc, format, &opts);
            EditableDocument::new(Arc::clone(doc), edit, facts.editability())
        };
        let stamps = Pair::new(docs.left.load_stamp(), docs.right.load_stamp());
        let mut docs = Pair::new(
            editable(Side::Left, &specs.left, &docs.left),
            editable(Side::Right, &specs.right, &docs.right),
        );
        // `--savetarget` redirects the right pane's in-place save (D-275); the left one saves in place.
        if let Some(raw) = &opts.save_target {
            let target = self.save_target_of(vfs, raw, &job.cancel)?;
            docs.right.set_save_target(Some(target));
        }
        let response = loaded.respond(req, Arc::clone(live.current()));
        Ok(LiveTextView {
            response,
            live,
            docs,
            specs,
            options: opts,
            stamps,
        })
    }

    /// The spec of a `--savetarget` (an absolute canonical spec, [`super::save_target_spec`]) on `vfs`: archives
    /// looked into (a member of a writable archive is a target); `-` and `clipboard://` are `InvalidArgument`.
    pub(crate) fn save_target_of(
        &self,
        vfs: &dyn Vfs,
        raw: &str,
        cancel: &hikasa_core::CancelToken,
    ) -> Result<Spec, ApiError> {
        let spec = Spec::parse_os(std::ffi::OsStr::new(raw), &self.spec_context())?;
        if matches!(spec, Spec::Stdin | Spec::Clipboard) {
            return Err(ApiError::InvalidArgument(format!(
                "`--savetarget` needs a file, not `{raw}`"
            )));
        }
        hikasa_vfs::resolve_spec(vfs, spec, hikasa_vfs::SpecUse::Any, cancel)
            .map_err(|_| ApiError::Cancelled)
    }
}

/// The facts of one loaded pane (§1.10.7): its location through `vfs.writable` (an error counts as "cannot
/// tell"), the format, the document and the options.
fn edit_facts(
    vfs: &dyn Vfs,
    side: Side,
    spec: &Spec,
    doc: &TextDocument,
    format: &CompiledFormat,
    opts: &LiveOptions,
) -> EditFacts {
    let has_location = !matches!(spec, Spec::Stdin | Spec::Clipboard);
    let writable = if has_location {
        vfs.writable(spec).ok().flatten()
    } else {
        None
    };
    EditFacts {
        load_failed: false,
        cancelled: false,
        has_location,
        archive_member: matches!(spec, Spec::ArchiveMember { .. }),
        writable,
        converted: converts_without_save(format),
        truncated: doc.truncated(),
        parent_read_only: opts.parent_read_only.get(side),
        session_read_only: opts.session_read_only.get(side),
        command_line: opts.read_only.get(side),
        binary: doc.binary().is_binary,
    }
}
