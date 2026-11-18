//! Text Edit and text pairs (M2 W10c; contract §1.10.21, TE-001, TC-064, D-289).
//!
//! ```text
//! text_edit_open(req, job)          one EditableDocument, no comparison: the spec read as a text compare reads it
//!                                   (format by D-15 or forced, the encoding override, External conversion); the save
//!                                   target resolved first and the §1.10.7 facts taken from it (W10b note 59); the
//!                                   format's and the settings' edit options → TextEditResponse
//! text_edit_session / …_request_of  a `text-edit` session: [specs] left (the file), left_format, left_encoding,
//!                                   read_only_left
//! text_compare_texts(l, r, req)     two named texts (the names choose the format) → documents from text, no location
//!                                   (Save is Save As), a live comparison like any other → LiveTextView
//! ```
//!
//! Editing, find and replace, conversions, bookmarks, Go To and saves work on the document as in a pane: the response's
//! `doc` is an [`EditableDocument`] and its saves go through [`Service::text_save_plan`] with no side
//! ([`Service::text_edit_save_request`] builds the request).

use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::{Pair, Side, Spec, Warning, WarningCode};
use hikasa_folder::ops::ReadOnlySides;
use hikasa_formats::CompiledFormat;
use hikasa_text::save::{SaveOptions, SaveRequest, UnmappablePolicy};
use hikasa_text::{
    Editability, EditableDocument, LoadStamp, TextDocument, compare_live, load_document,
};
use hikasa_vfs::Vfs;
use toml::Table;

use super::compare::{LoadedPair, check_anchors, check_ignored, load_failed};
use super::requests::TextCompareRequest;
use super::responses::FormatChoice;
use super::text_live::{EditFacts, LiveOptions, LiveTextView, edit_facts, edit_options};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};
use crate::sessions::{ParentLink, Session, SessionRef, SessionRules, SessionType, Specs};

/// What a Text Edit view opens (TE-001, §1.10.21): one file with its options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextEditRequest {
    /// The file (a path, an archive member, `-`, `clipboard://`).
    pub spec: OsString,
    /// The format, an id or a name (`None`: detected from the name, D-15).
    pub format: Option<String>,
    /// The encoding label (`None`: detected).
    pub encoding: Option<String>,
    /// `--ro`, `--ro1` (a Text Edit view's file is side 1, §1.10.22): `CommandLine`.
    pub read_only: bool,
    /// `--savetarget=<file>`: an absolute canonical spec an in-place save writes instead (§1.10.14, D-275).
    pub save_target: Option<String>,
    /// The session's `[specs] read_only_left`: `SessionReadOnly`. Additive.
    pub session_read_only: bool,
    /// The parent folder comparison's side is read-only (D-184): `ParentReadOnly`. Additive.
    pub parent_read_only: bool,
    /// `--nobackups`: no backup on any save of this view ([`Service::text_edit_save_request`]). Additive.
    pub no_backups: bool,
}

/// An open Text Edit document (§1.10.21).
#[derive(Debug)]
pub struct TextEditResponse {
    /// The document, with its editability, its save target and its splice source.
    pub doc: EditableDocument,
    /// Its editability (§1.10.7), as the document holds it at the open.
    pub editability: Editability,
    /// The load stamp (size, time, CRC-32 of the bytes read; `None` for standard input and the clipboard).
    pub stamp: Option<LoadStamp>,
    /// The resolved spec of the file.
    pub spec: Spec,
    /// The format and why it was chosen.
    pub format: FormatChoice,
    /// The format itself: its edit and *when saving* options, its External save program.
    pub compiled: Arc<CompiledFormat>,
    /// The request it was opened with (the save target and `--nobackups` for the saves).
    pub request: TextEditRequest,
    /// The format's warnings and the document's (`binary_content`, `lossy_decode`).
    pub warnings: Vec<Warning>,
}

impl TextEditResponse {
    /// The format's *when saving* options (docs/10 §1), applied by the adapter before it plans a save.
    pub fn save_options(&self) -> SaveOptions {
        SaveOptions::of(&self.compiled)
    }
}

/// One text of a text pair (TC-064's Compare Selection to Clipboard, W11's Open Clipboard and dropped text).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedText {
    /// The name the format is detected from (a file name, or empty: none).
    pub name: String,
    /// The text.
    pub text: String,
}

/// The document's own warnings of a Text Edit (a comparison gives them for its panes).
fn document_warnings(doc: &TextDocument) -> Vec<Warning> {
    let spec = doc.origin().spec.as_ref().map(Spec::canonical);
    let mut warnings = Vec::new();
    if doc.binary().is_binary {
        warnings.push(Warning {
            code: WarningCode::BinaryContent,
            message: format!(
                "{}: binary content; editing it needs a confirmation",
                doc.origin().display()
            ),
            spec: spec.clone(),
        });
    }
    if doc.malformed() > 0 {
        warnings.push(Warning {
            code: WarningCode::LossyDecode,
            message: format!(
                "{}: {} malformed byte sequence(s) replaced with U+FFFD while decoding as {}",
                doc.origin().display(),
                doc.malformed(),
                doc.encoding().encoding
            ),
            spec,
        });
    }
    warnings
}

/// `""` → `None`.
fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

impl Service {
    /// Opens a Text Edit document (TE-001, §1.10.21; run as a job, `JobKind::TextCompare`): one
    /// [`EditableDocument`], no comparison. The spec is read as `text compare` reads a side (the format forced by
    /// `format` or detected from the name, D-15; the encoding override; an External conversion); `--savetarget` is
    /// resolved first and the document's §1.10.7 facts are the target's when it has one (W10b note 59: a read-only
    /// file saved to a writable target is editable, a read-only target makes it read-only); the blocks of
    /// `read_only` (`CommandLine`), `session_read_only` and `parent_read_only` as in a pane. A file that cannot be
    /// loaded is `LoadFailed` (exit 100).
    pub fn text_edit_open(
        &self,
        req: &TextEditRequest,
        job: &JobControl,
    ) -> Result<TextEditResponse, ApiError> {
        run_guarded(|| self.text_edit_open_inner(None, req, job))
    }

    /// [`Service::text_edit_open`] on the back-end `vfs` (a test's `MemoryVfs`, as `text_compare_live_on`).
    pub fn text_edit_open_on(
        &self,
        vfs: &dyn Vfs,
        req: &TextEditRequest,
        job: &JobControl,
    ) -> Result<TextEditResponse, ApiError> {
        run_guarded(|| self.text_edit_open_inner(Some(vfs), req, job))
    }

    fn text_edit_open_inner(
        &self,
        on: Option<&dyn Vfs>,
        req: &TextEditRequest,
        job: &JobControl,
    ) -> Result<TextEditResponse, ApiError> {
        let spec = self.parse_input(on, &req.spec, &job.cancel)?;
        let router = self.router();
        let vfs: &dyn Vfs = on.unwrap_or(&*router);
        vfs.prepare(&spec, &job.ctl())
            .map_err(|source| load_failed(Side::Left, &spec, source))?;
        let mut warnings = Vec::new();
        let name = self.detection_name(&spec);
        let overrides = self.enabled_overrides(&std::collections::BTreeMap::new(), &mut warnings);
        let chosen = self.choose_format_with(
            req.format.as_deref(),
            Pair::new(name.as_deref(), None),
            &overrides,
        )?;
        warnings.extend(chosen.warnings.iter().cloned());
        let load = self.load_options(req.encoding.as_deref())?;
        let (doc, raw) = load_document(
            vfs,
            &spec,
            &chosen.format,
            &load,
            Some(self.external()),
            &job.ctl(),
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, &spec.canonical()))?;
        drop(raw);
        warnings.extend(document_warnings(&doc));
        let doc = Arc::new(doc);
        let save_target = match &req.save_target {
            Some(raw) => Some(self.save_target_of(vfs, raw, &job.cancel)?),
            None => None,
        };
        let opts = LiveOptions {
            read_only: ReadOnlySides {
                left: req.read_only,
                right: false,
            },
            parent_read_only: ReadOnlySides {
                left: req.parent_read_only,
                right: false,
            },
            session_read_only: ReadOnlySides {
                left: req.session_read_only,
                right: false,
            },
            save_target: req.save_target.clone(),
            no_backups: req.no_backups,
        };
        let writes = save_target.as_ref().unwrap_or(&spec);
        let facts = edit_facts(vfs, Side::Left, writes, &doc, &chosen.format, &opts);
        let edit = edit_options(&chosen.format, &self.fresh_options(), self.env().platform);
        let stamp = doc.load_stamp();
        let mut editable = EditableDocument::new(doc, edit, facts.editability());
        editable.set_save_target(save_target);
        Ok(TextEditResponse {
            editability: editable.editability(),
            doc: editable,
            stamp,
            spec,
            format: chosen.choice.clone(),
            compiled: Arc::clone(&chosen.format),
            request: req.clone(),
            warnings,
        })
    }

    /// The request of a Text Edit save (§1.10.14): in place (or to the `--savetarget` the document carries), backups
    /// unless opened with `--nobackups`, the unmappable characters asked, the format's External save program (FF-030).
    /// Planned with no side: `text_save_plan(&[(None, &resp.doc, &request)], job)`.
    pub fn text_edit_save_request(&self, resp: &TextEditResponse) -> Result<SaveRequest, ApiError> {
        Ok(SaveRequest {
            target: None,
            backup: self.backup_policy_for_save(resp.request.no_backups)?,
            unmappable: UnmappablePolicy::Ask,
            conversion: SaveRequest::conversion_of(&resp.compiled),
            transform: None,
        })
    }

    /// The current session of a Text Edit view (D-120's rule for `session_for_view`, session type `text-edit`, D-80):
    /// `[specs] left` is the file as stored (`-` and `clipboard://` stay empty with `spec_not_saved`), `left_format`,
    /// `left_encoding`, `read_only_left` (the session's own flag, not `--ro`); the type's default rules; the view's
    /// `view` and `display` tables and its parent link.
    pub fn text_edit_session(
        &self,
        req: &TextEditRequest,
        view: Table,
        display: Table,
        parent: Option<ParentLink>,
    ) -> Result<(Session, Vec<Warning>), ApiError> {
        run_guarded(|| {
            let mut warnings = Vec::new();
            let (left, warning) = self.stored_spec(&req.spec.to_string_lossy())?;
            warnings.extend(warning);
            let defaults = self.session_store().defaults(SessionType::TextEdit)?;
            let rules = match defaults.rules {
                rules @ SessionRules::Text(_) => rules,
                _ => SessionType::TextEdit.builtin_rules(),
            };
            Ok((
                Session {
                    specs: Specs {
                        left,
                        left_format: req.format.clone().unwrap_or_default(),
                        left_encoding: req.encoding.clone().unwrap_or_default(),
                        read_only_left: req.session_read_only,
                        ..Specs::default()
                    },
                    rules,
                    view,
                    display,
                    parent,
                    ..Session::builtin(SessionType::TextEdit)
                },
                warnings,
            ))
        })
    }

    /// The Text Edit request of a `text-edit` session: `[specs] left`, `left_format` (else `format`), `left_encoding`,
    /// `read_only_left` as `session_read_only`. Another type, or a session without a file, is `InvalidArgument`.
    pub fn text_edit_request_of(&self, session: &Session) -> Result<TextEditRequest, ApiError> {
        if session.kind != SessionType::TextEdit {
            return Err(ApiError::InvalidArgument(format!(
                "a Text Edit opens a text-edit session, not a {} session",
                session.kind
            )));
        }
        let specs = &session.specs;
        if specs.left.is_empty() {
            return Err(ApiError::InvalidArgument(
                "the text-edit session has no file ([specs] left)".to_owned(),
            ));
        }
        Ok(TextEditRequest {
            spec: specs.left.clone().into(),
            format: non_empty(&specs.left_format).or_else(|| non_empty(&specs.format)),
            encoding: non_empty(&specs.left_encoding),
            session_read_only: specs.read_only_left,
            ..TextEditRequest::default()
        })
    }

    /// The Text Edit request of a named `text-edit` session ([`Service::text_edit_request_of`] of its effective
    /// session): what the GUI opens for it.
    pub fn session_text_edit(&self, reference: &SessionRef) -> Result<TextEditRequest, ApiError> {
        run_guarded(|| {
            let session = self.session_effective(reference)?;
            self.text_edit_request_of(&session)
        })
    }

    /// Compares two texts (§1.10.21's text pairs, TC-064; run as a job, `JobKind::TextCompare`): each text becomes a
    /// document by `TextDocument::from_text` (a text over the D-43 limits is cut, `Truncated`), the formats are
    /// chosen from the names (D-15; `req.format` and `req.formats` force them), and then the comparison runs as any
    /// live one: the request's rules, anchors and ignored lines. The documents have no location (`clipboard://` stands
    /// for them in `specs`): they are editable and Save is Save As. `req.left` and `req.right` are not read.
    pub fn text_compare_texts(
        &self,
        left: NamedText,
        right: NamedText,
        req: &TextCompareRequest,
        job: &JobControl,
    ) -> Result<LiveTextView, ApiError> {
        run_guarded(|| self.text_compare_texts_inner(&left, &right, req, job))
    }

    fn text_compare_texts_inner(
        &self,
        left: &NamedText,
        right: &NamedText,
        req: &TextCompareRequest,
        job: &JobControl,
    ) -> Result<LiveTextView, ApiError> {
        let mut warnings = Vec::new();
        let rules = self.text_rules_from(
            req.base_rules.as_ref(),
            req.rules_file.as_deref(),
            &req.rules_patch,
            &mut warnings,
        )?;
        let names = Pair::new(non_empty(&left.name), non_empty(&right.name));
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let base = self.choose_format_with(
            req.format.as_deref(),
            names.as_ref().map(Option::as_deref),
            &overrides,
        )?;
        let (chosen, right_chosen) = self.per_side_formats(req, base)?;
        warnings.extend(chosen.warnings.iter().cloned());
        let left_format = Arc::clone(&chosen.format);
        let right_format = right_chosen
            .as_ref()
            .map_or_else(|| Arc::clone(&left_format), |c| Arc::clone(&c.format));
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        // Each pane is wrapped at its format's characters-per-line limit by `EditableDocument::new`; the comparison reads
        // the panes' snapshots, so rows and documents agree.
        let fresh = self.fresh_options();
        let pane = |text: &NamedText, format: &Arc<CompiledFormat>| {
            let doc = TextDocument::from_text(&text.text);
            let facts = EditFacts {
                has_location: false,
                truncated: doc.truncated(),
                binary: doc.binary().is_binary,
                ..EditFacts::default()
            };
            EditableDocument::new(
                Arc::new(doc),
                edit_options(format, &fresh, self.env().platform),
                facts.editability(),
            )
        };
        let docs = Pair::new(pane(left, &left_format), pane(right, &right_format));
        let snapshots = Pair::new(docs.left.snapshot(), docs.right.snapshot());
        check_anchors(&req.anchors, &snapshots.left, &snapshots.right)?;
        let specs = Pair::new(Spec::Clipboard, Spec::Clipboard);
        let ignored = check_ignored(
            req,
            Pair::new(&snapshots.left, &snapshots.right),
            Pair::new(&specs.left, &specs.right),
            &mut warnings,
        )?;
        let loaded = LoadedPair {
            specs: specs.clone(),
            docs: snapshots,
            chosen,
            right_chosen,
            ignored,
            rules,
            bytes_identical: left.text == right.text,
            warnings,
        };
        let ctl = job.ctl();
        let live = compare_live(
            loaded.compare_input(req),
            self.registry().limits(),
            Some(self.pool()),
            &ctl,
            None,
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, "clipboard://"))?;
        let response = loaded.respond(req, Arc::clone(live.current()));
        Ok(LiveTextView {
            response,
            live,
            docs,
            specs,
            options: LiveOptions::default(),
            stamps: Pair::new(None, None),
        })
    }
}
