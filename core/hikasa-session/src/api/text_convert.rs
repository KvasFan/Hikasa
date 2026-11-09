//! `hikasa text convert` (TC-063; D-279, D-280; contract §1.10.15, §1.10.24): conversions of whole files, written
//! through the save plan of §1.10.13 so unchanged lines stay byte-exact.
//!
//! ```text
//! text_convert_plan(req, job)        per file: the spec; the format (--format, else detection by name) and the
//!                                    encoding (--encoding-in, else detection), loaded as a text compare loads it; the
//!                                    refusals (binary, a conversion without a save path, truncated, an archive that
//!                                    cannot be written); the conversions in the fixed order trim, tabs, line endings,
//!                                    encoding/BOM (`transform` + `SetEncoding`, one EditableDocument per file); a file
//!                                    with nothing to change is skipped (`no_change`) and never rewritten; the others
//!                                    are one save plan (in place, `--output`, or stdout: the in-place plan's content,
//!                                    streamed and never written)
//! text_convert_confirmation(&plan)   the save questions (SaveDriver: --yes, --force, --unmappable replace)
//! text_convert_dry_run(plan)         nothing written
//! text_convert_execute(plan, out)    execute_save (in place, --output) or the content streamed to `out` (stdout)
//!   → TextConvertOutcome { dto: hikasa.text_convert.v1, warnings, exit_code }
//! text_convert(req, out, job)        the four steps with the request's answers and no person
//! ```
//!
//! Exit codes: 0 when every file is done or skipped, 100 otherwise (also after a cancel). A request that cannot run
//! (no conversion flag, several files to stdout, `--output` with several files, an unknown label or format, a tab
//! size outside 1–32, open questions nobody answers) is an error, nothing is written.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use hikasa_core::{ErrorKind, ErrorReport, ExitCode, Pair, Spec, Warning};
use hikasa_formats::{FormatError, TextEncoding};
use hikasa_report::json::FormatJson;
use hikasa_text::save::{SavePlan, SaveQuestion, SaveRequest, SaveSource, UnmappablePolicy};
use hikasa_text::{
    EditBlock, EditRequest, Editability, EditableDocument, EolStyle, GroupHint, LineEnding,
    LineIdx, LineMark, Transform, load_document, transform,
};
use hikasa_vfs::ops::{ItemStatus, SkipReason};
use hikasa_vfs::{SpecUse, Vfs};
use serde::Serialize;

use super::text_edit::{SaveAnswers, SaveDriver, SavePlanItemDto, save_plan_items};
use super::text_live::{converts_without_save, edit_options};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// Where `text convert` writes (§1.10.15).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConvertTarget {
    /// The converted bytes of the one file go to the caller's output; no plan is written, no backup.
    #[default]
    Stdout,
    /// Each file is rewritten: one plan for all, backups by `back_up_before_save` unless `--no-backup`.
    InPlace,
    /// `--output <FILE>` (one input): the file written; an existing one asks `ReplaceExisting`.
    Output(OsString),
}

impl ConvertTarget {
    /// `stdout`, `in_place`, `output` (JSON `target`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ConvertTarget::Stdout => "stdout",
            ConvertTarget::InPlace => "in_place",
            ConvertTarget::Output(_) => "output",
        }
    }
}

/// The tab conversion of `text convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabsConversion {
    /// `--tabs-to-spaces`.
    ToSpaces,
    /// `--spaces-to-tabs`: leading spaces that reach a tab stop.
    ToTabs,
}

/// `hikasa text convert` (§1.10.15). Words (`--line-endings`, `--unmappable`, `--lines`) are parsed by
/// [`super::vocab`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextConvertRequest {
    /// The files (`-` is standard input: stdout and `--output` only).
    pub files: Vec<OsString>,
    /// `--trim-trailing`.
    pub trim_trailing: bool,
    /// `--tabs-to-spaces` / `--spaces-to-tabs`.
    pub tabs: Option<TabsConversion>,
    /// `--tab-size <1-32>` (default: the format's).
    pub tab_size: Option<u8>,
    /// `--line-endings lf|crlf|cr`.
    pub line_endings: Option<LineEnding>,
    /// `--encoding <label>`: the encoding written.
    pub encoding: Option<String>,
    /// `--bom` (`Some(true)`) / `--no-bom` (`Some(false)`); `None` keeps the file's (when the encoding has one).
    pub bom: Option<bool>,
    /// `--lines <a>-<b>`: 1-based lines of the file (the content conversions only).
    pub lines: Option<RangeInclusive<u32>>,
    /// `--format <id|name>`.
    pub format: Option<String>,
    /// `--encoding-in <label>`: the encoding the files are read in.
    pub encoding_in: Option<String>,
    /// What an unmappable character does (`Ask` is a question in a plan; standard output never asks: it refuses).
    pub unmappable: UnmappablePolicy,
    /// Where the result goes.
    pub target: ConvertTarget,
    /// `--backup` / `--no-backup`; `None`: `back_up_before_save`.
    pub backup: Option<bool>,
    /// The flags that answer questions (`--yes`, `--force`, `--confirm`).
    pub answers: SaveAnswers,
    /// `--dry-run`.
    pub dry_run: bool,
}

impl TextConvertRequest {
    /// True when a conversion flag is given (`--trim-trailing`, a tab conversion, `--line-endings`, `--encoding`,
    /// `--bom`, `--no-bom`).
    pub fn has_conversion(&self) -> bool {
        self.trim_trailing
            || self.tabs.is_some()
            || self.line_endings.is_some()
            || self.encoding.is_some()
            || self.bom.is_some()
    }
}

/// `{from, to}` of a file's encoding, BOM or line endings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FromTo<T> {
    /// As loaded.
    pub from: T,
    /// As written.
    pub to: T,
}

/// One conversion of a file and the lines it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversionDto {
    /// `trim_trailing`, `spaces_to_tabs`, `tabs_to_spaces`, `line_endings`, `encoding`, `bom`.
    pub kind: &'static str,
    /// Physical lines changed (`encoding`: every line when the encoding changes; `bom`: 0).
    pub lines: u64,
}

/// One file of `hikasa.text_convert.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConvertFileDto {
    /// The file's canonical spec.
    pub spec: String,
    /// Its format (`null` when its spec could not be read).
    pub format: Option<FormatJson>,
    /// The encoding read and written (`null`: not loaded).
    pub encoding: FromTo<Option<&'static str>>,
    /// The BOM read and written.
    pub bom: FromTo<Option<bool>>,
    /// The line endings read and written.
    pub line_endings: FromTo<Option<EolStyle>>,
    /// The conversions asked, in their fixed order.
    pub conversions: Vec<ConversionDto>,
    /// Physical lines whose bytes change.
    pub converted_lines: u64,
    /// `planned`, `done`, `skipped`, `failed`, `not_run`.
    pub status: ItemStatus,
    /// The skip reason (`no_change`, `declined`).
    pub skip: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
    /// The save questions of the file (`changed_on_disk`, …).
    pub questions: Vec<&'static str>,
    /// The file's warnings.
    pub warnings: Vec<Warning>,
}

/// The counts of `hikasa.text_convert.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ConvertSummaryDto {
    /// Files named.
    pub files: u64,
    /// Lines converted in the files done (planned, in a dry run).
    pub converted_lines: u64,
    /// Files written.
    pub done: u64,
    /// Files skipped.
    pub skipped: u64,
    /// Files failed.
    pub failed: u64,
    /// Files not reached (a cancel).
    pub not_run: u64,
}

/// `hikasa.text_convert.v1` (`result`, §1.10.24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextConvertDto {
    /// Nothing was written.
    pub dry_run: bool,
    /// `in_place`, `output`, `stdout`.
    pub target: &'static str,
    /// One per file named, in order.
    pub files: Vec<ConvertFileDto>,
    /// The save plan, one item per file (empty for stdout).
    pub plan: Vec<SavePlanItemDto>,
    /// The counts.
    pub summary: ConvertSummaryDto,
}

/// What `text convert` did, or would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextConvertOutcome {
    /// `hikasa.text_convert.v1`.
    pub dto: TextConvertDto,
    /// Every warning (load, plan, execution), for the envelope.
    pub warnings: Vec<Warning>,
    /// 0 when every file is done or skipped (planned or skipped in a dry run), else 100.
    pub exit_code: ExitCode,
    /// The plan's human lines (`write <target> (…)`), for `--verbose`.
    pub plan_lines: Vec<String>,
}

impl TextConvertOutcome {
    /// One line per file (§1.10.15): `converted <n> lines: <spec>` (`would convert …` in a dry run), `no change:
    /// <spec>`, `skipped (<reason>): <spec>`, `failed: <spec>: <message>`, `not run: <spec>`.
    pub fn human_lines(&self) -> Vec<String> {
        let dry = self.dto.dry_run;
        self.dto
            .files
            .iter()
            .map(|file| match file.status {
                ItemStatus::Done | ItemStatus::Planned => format!(
                    "{} {} lines: {}",
                    if dry { "would convert" } else { "converted" },
                    file.converted_lines,
                    file.spec
                ),
                ItemStatus::Skipped if file.skip == Some(SkipReason::NoChange) => {
                    format!("no change: {}", file.spec)
                }
                ItemStatus::Skipped => format!(
                    "skipped ({}): {}",
                    file.skip.map_or("skipped", SkipReason::as_str),
                    file.spec
                ),
                ItemStatus::Failed => format!(
                    "failed: {}: {}",
                    file.spec,
                    file.error
                        .as_ref()
                        .map_or("failed", |error| error.message.as_str())
                ),
                ItemStatus::NotRun => format!("not run: {}", file.spec),
            })
            .collect()
    }
}

/// A file's state before the save plan.
#[derive(Debug, Clone)]
enum Pre {
    /// Refused or not loaded.
    Failed(ErrorReport),
    /// Nothing to change.
    NoChange,
    /// In the save plan at this item.
    Planned(usize),
}

/// One file of a plan.
#[derive(Debug, Clone)]
struct ConvertFile {
    dto: ConvertFileDto,
    pre: Pre,
}

/// A planned `text convert` (§1.10.15): the files, their converted documents and the save plan of those that change.
#[derive(Debug, Clone)]
pub struct TextConvertPlan {
    target: ConvertTarget,
    dry_run: bool,
    answers: SaveAnswers,
    files: Vec<ConvertFile>,
    /// The documents of the save plan's items, in its order.
    docs: Vec<EditableDocument>,
    save: Option<SavePlan>,
    warnings: Vec<Warning>,
}

impl TextConvertPlan {
    /// The save plan (`None` when no file changes).
    pub fn save_plan(&self) -> Option<&SavePlan> {
        self.save.as_ref()
    }

    /// The converted documents, in the save plan's order (the exactness tests compare them with a view's).
    pub fn documents(&self) -> &[EditableDocument] {
        &self.docs
    }

    /// The plan with the answers of `driver` ([`SaveDriver::finish`]): declined items are skipped; a cancel is
    /// `Cancelled`.
    pub fn answered(mut self, driver: SaveDriver) -> Result<TextConvertPlan, ApiError> {
        if let Some(save) = self.save.take() {
            self.save = Some(driver.finish(save).map_err(|_| ApiError::Cancelled)?);
        }
        Ok(self)
    }
}

/// The physical lines of `doc` holding an edited piece (the lines a conversion changed).
fn edited_lines(doc: &EditableDocument) -> u64 {
    let mut count = 0;
    let mut edited = false;
    for l in 0..doc.line_count() {
        edited |= doc.mark(LineIdx(l)) == LineMark::Edited;
        if doc.line_ending(LineIdx(l)) != LineEnding::Artificial {
            count += u64::from(edited);
            edited = false;
        }
    }
    count + u64::from(edited)
}

/// The physical lines of `doc`.
fn physical_lines(doc: &EditableDocument) -> u64 {
    (0..doc.line_count())
        .filter(|&l| doc.line_ending(LineIdx(l)) != LineEnding::Artificial)
        .count() as u64
        + u64::from(
            doc.line_count() > 0
                && doc.line_ending(LineIdx(doc.line_count() - 1)) == LineEnding::Artificial,
        )
}

/// The report of an item error.
fn item_error(kind: ErrorKind, spec: &str, message: String) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: Some(spec.to_owned()),
        causes: Vec::new(),
    }
}

/// The report of a refusal of `--bom` for an encoding without a byte-order mark.
fn no_bom(spec: &str, encoding: TextEncoding) -> ErrorReport {
    item_error(
        ErrorKind::InvalidInput,
        spec,
        format!("{} has no byte-order mark", encoding.name()),
    )
}

/// The conversion steps of a request in their fixed order (trim, tabs, line endings).
fn steps(req: &TextConvertRequest) -> Vec<(&'static str, Transform)> {
    let mut steps = Vec::new();
    if req.trim_trailing {
        steps.push(("trim_trailing", Transform::TrimTrailing));
    }
    match req.tabs {
        Some(TabsConversion::ToSpaces) => steps.push(("tabs_to_spaces", Transform::TabsToSpaces)),
        Some(TabsConversion::ToTabs) => {
            steps.push(("spaces_to_tabs", Transform::LeadingSpacesToTabs));
        }
        None => {}
    }
    if let Some(eol) = req.line_endings {
        steps.push(("line_endings", Transform::LineEndings(eol)));
    }
    steps
}

/// A file as loaded and converted.
struct Converted {
    doc: EditableDocument,
    format: Arc<hikasa_formats::CompiledFormat>,
}

impl Service {
    /// Plans `text convert` (§1.10.15): see the module documentation. Nothing is written. Refuses a request that
    /// cannot run (no conversion flag, several files with standard output, `--output` with several files, an unknown
    /// `--encoding`/`--encoding-in` label or `--format`, a tab size outside 1–32); a file that cannot be loaded or
    /// converted fails its item.
    pub fn text_convert_plan(
        &self,
        req: &TextConvertRequest,
        job: &JobControl,
    ) -> Result<TextConvertPlan, ApiError> {
        run_guarded(|| self.text_convert_plan_inner(req, job))
    }

    fn check_convert(req: &TextConvertRequest) -> Result<(), ApiError> {
        if !req.has_conversion() {
            return Err(ApiError::InvalidArgument(
                "nothing to convert: pass --trim-trailing, --tabs-to-spaces, --spaces-to-tabs, --line-endings, \
                 --encoding, --bom or --no-bom"
                    .to_owned(),
            ));
        }
        if req.files.is_empty() {
            return Err(ApiError::InvalidArgument("no file to convert".to_owned()));
        }
        match &req.target {
            ConvertTarget::Stdout if req.files.len() > 1 => {
                return Err(ApiError::InvalidArgument(
                    "several files cannot go to standard output: pass --in-place, or one file"
                        .to_owned(),
                ));
            }
            ConvertTarget::Output(_) if req.files.len() > 1 => {
                return Err(ApiError::InvalidArgument(
                    "--output takes one input file".to_owned(),
                ));
            }
            _ => {}
        }
        if let Some(size) = req.tab_size
            && !(1..=32).contains(&size)
        {
            return Err(ApiError::InvalidArgument(format!(
                "--tab-size {size} is out of range (1-32)"
            )));
        }
        for label in [&req.encoding, &req.encoding_in].into_iter().flatten() {
            if TextEncoding::for_label(label).is_none() {
                return Err(ApiError::Format(FormatError::UnknownEncoding {
                    label: label.clone(),
                }));
            }
        }
        Ok(())
    }

    fn text_convert_plan_inner(
        &self,
        req: &TextConvertRequest,
        job: &JobControl,
    ) -> Result<TextConvertPlan, ApiError> {
        Self::check_convert(req)?;
        if let Some(forced) = &req.format {
            // An unknown format refuses the request, before any file is read.
            self.registry().force(forced)?;
        }
        let output = match &req.target {
            ConvertTarget::Output(raw) => Some(self.parse_spec_as(raw, SpecUse::Any)?),
            _ => None,
        };
        let backup = match req.target {
            ConvertTarget::Stdout => None,
            ConvertTarget::InPlace | ConvertTarget::Output(_) => {
                self.backup_policy_for_save_with(req.backup)?
            }
        };
        let unmappable = match (&req.target, req.unmappable) {
            (ConvertTarget::Stdout, UnmappablePolicy::Ask) => UnmappablePolicy::Refuse,
            (_, policy) => policy,
        };
        let options = self.fresh_options();
        let router = self.router();
        let vfs: &dyn Vfs = &*router;
        let mut files = Vec::with_capacity(req.files.len());
        let mut docs: Vec<(EditableDocument, SaveRequest)> = Vec::new();
        let mut warnings = Vec::new();
        for raw in &req.files {
            job.cancel.check().map_err(|_| ApiError::Cancelled)?;
            let mut file = ConvertFileDto {
                spec: raw.to_string_lossy().into_owned(),
                format: None,
                encoding: FromTo {
                    from: None,
                    to: None,
                },
                bom: FromTo {
                    from: None,
                    to: None,
                },
                line_endings: FromTo {
                    from: None,
                    to: None,
                },
                conversions: Vec::new(),
                converted_lines: 0,
                status: ItemStatus::Failed,
                skip: None,
                error: None,
                questions: Vec::new(),
                warnings: Vec::new(),
            };
            let pre = match self.convert_one(vfs, req, raw, &options, &mut file, job)? {
                Err(error) => Pre::Failed(error),
                Ok(None) => Pre::NoChange,
                Ok(Some(converted)) => {
                    if req.bom == Some(true) && converted.doc.encoding().0.bom().is_empty() {
                        Pre::Failed(no_bom(&file.spec, converted.doc.encoding().0))
                    } else {
                        let target = match (&req.target, &output) {
                            (_, Some(output)) => Some(output.clone()),
                            (ConvertTarget::Stdout, None) if !converted.doc.can_save_in_place() => {
                                Some(self.stdout_spec(job)?)
                            }
                            _ => None,
                        };
                        let save = SaveRequest {
                            target,
                            backup: backup.clone(),
                            unmappable,
                            conversion: SaveRequest::conversion_of(&converted.format),
                        };
                        docs.push((converted.doc, save));
                        Pre::Planned(docs.len() - 1)
                    }
                }
            };
            warnings.extend(file.warnings.iter().cloned());
            files.push(ConvertFile { dto: file, pre });
        }
        let save = if docs.is_empty() {
            None
        } else {
            let triples: Vec<_> = docs.iter().map(|(doc, save)| (None, doc, save)).collect();
            Some(self.plan_save_in(vfs, &triples, job)?)
        };
        let docs = docs.into_iter().map(|(doc, _)| doc).collect();
        Ok(TextConvertPlan {
            target: match &req.target {
                ConvertTarget::Output(_) => ConvertTarget::Output(
                    output
                        .as_ref()
                        .map(|s| OsString::from(s.canonical()))
                        .unwrap_or_default(),
                ),
                other => other.clone(),
            },
            dry_run: req.dry_run,
            answers: req.answers,
            files,
            docs,
            save,
            warnings,
        })
    }

    /// The spec a standard-input document is planned to for standard output: a file of the temporary folder the
    /// plan never writes (its content is streamed).
    fn stdout_spec(&self, job: &JobControl) -> Result<Spec, ApiError> {
        let path = self
            .env()
            .temp_dir
            .join(format!("hikasa-convert-{}.out", job.id.0));
        Ok(Spec::parse_os(path.as_os_str(), &self.spec_context())?)
    }

    /// Loads and converts one file: `Ok(Err(_))` a failure of the file, `Ok(Ok(None))` nothing to change, `Err` a
    /// cancel.
    #[allow(clippy::type_complexity)]
    fn convert_one(
        &self,
        vfs: &dyn Vfs,
        req: &TextConvertRequest,
        raw: &std::ffi::OsStr,
        options: &crate::settings::options::Options,
        file: &mut ConvertFileDto,
        job: &JobControl,
    ) -> Result<Result<Option<Converted>, ErrorReport>, ApiError> {
        let ctl = job.ctl();
        let spec = match self.parse_input(None, raw, &job.cancel) {
            Ok(spec) => spec,
            Err(ApiError::Cancelled) => return Err(ApiError::Cancelled),
            Err(e) => return Ok(Err(e.report())),
        };
        file.spec = spec.canonical();
        let name = self.detection_name(&spec);
        let chosen =
            match self.choose_format(req.format.as_deref(), Pair::new(name.as_deref(), None)) {
                Ok(chosen) => chosen,
                Err(e) => return Ok(Err(e.report())),
            };
        file.format = Some(chosen.choice.json());
        let format = Arc::clone(&chosen.format);
        let fail = |e: ApiError| -> Result<Result<Option<Converted>, ErrorReport>, ApiError> {
            match e {
                ApiError::Cancelled => Err(ApiError::Cancelled),
                e => {
                    let mut report = e.report();
                    report.spec.get_or_insert_with(|| spec.canonical());
                    Ok(Err(report))
                }
            }
        };
        if let Err(source) = vfs.prepare(&spec, &ctl) {
            return fail(ApiError::from_vfs(source));
        }
        let load = self.load_options(req.encoding_in.as_deref())?;
        let doc = match load_document(vfs, &spec, &format, &load, Some(self.external()), &ctl) {
            Ok((doc, raw_bytes)) => {
                drop(raw_bytes);
                doc
            }
            Err(e) => {
                return fail(ApiError::from_text(
                    e,
                    hikasa_core::Side::Left,
                    &spec.canonical(),
                ));
            }
        };
        let (from_enc, from_bom) = (doc.encoding().encoding, doc.encoding().bom_len > 0);
        file.encoding.from = Some(from_enc.name());
        file.bom.from = Some(from_bom);
        file.line_endings.from = Some(doc.eol().style());
        // The refusals of §1.10.15, in the words of §1.10.7's blocks.
        let has_location = !matches!(spec, Spec::Stdin | Spec::Clipboard);
        let block = if doc.binary().is_binary {
            Some(EditBlock::Binary)
        } else if converts_without_save(&format) {
            Some(EditBlock::Converted)
        } else if doc.truncated() {
            Some(EditBlock::Truncated)
        } else if req.target != ConvertTarget::Stdout
            && matches!(spec, Spec::ArchiveMember { .. })
            && vfs.writable(&spec).ok().flatten() == Some(false)
        {
            Some(EditBlock::ArchiveReadOnly)
        } else {
            None
        };
        if let Some(block) = block {
            let mut report = ApiError::EditBlocked { side: None, block }.report();
            report.spec = Some(spec.canonical());
            return Ok(Err(report));
        }
        // Lines are the file's: no characters-per-line wrapping (a conversion takes whole physical lines anyway).
        let mut edit = edit_options(&format, options, self.env().platform);
        edit.chars_per_line_limit = None;
        let mut doc = EditableDocument::new(
            Arc::new(doc),
            edit,
            Editability::with_blocks([], has_location),
        );
        let tab_size = req.tab_size.unwrap_or(format.text().tab_size);
        let lines: Option<Range<u32>> = req.lines.as_ref().map(|r| (r.start() - 1)..*r.end());
        for (kind, step) in steps(req) {
            let out = transform(&doc, step, lines.clone(), tab_size);
            if let Some(request) = &out.request
                && let Err(e) = doc.apply(request, GroupHint::None)
            {
                return Ok(Err(item_error(
                    ErrorKind::InvalidInput,
                    &spec.canonical(),
                    e.to_string(),
                )));
            }
            file.conversions.push(ConversionDto {
                kind,
                lines: out.lines,
            });
        }
        // The encoding and the BOM last.
        let encoding = req
            .encoding
            .as_deref()
            .and_then(TextEncoding::for_label)
            .unwrap_or(from_enc);
        let bom = match req.bom {
            Some(bom) => bom,
            None => from_bom && !encoding.bom().is_empty(),
        };
        let changed_encoding = encoding != from_enc;
        if req.bom == Some(true) && encoding.bom().is_empty() {
            file.encoding.to = Some(encoding.name());
            file.bom.to = Some(false);
            file.line_endings.to = Some(doc.document().eol().style());
            return Ok(Err(no_bom(&spec.canonical(), encoding)));
        }
        if (encoding, bom) != doc.encoding() {
            let request = EditRequest::SetEncoding { encoding, bom };
            if let Err(e) = doc.apply(&request, GroupHint::None) {
                return Ok(Err(item_error(
                    ErrorKind::InvalidInput,
                    &spec.canonical(),
                    e.to_string(),
                )));
            }
        }
        if req.encoding.is_some() {
            file.conversions.push(ConversionDto {
                kind: "encoding",
                lines: if changed_encoding {
                    physical_lines(&doc)
                } else {
                    0
                },
            });
        }
        if req.bom.is_some() {
            file.conversions.push(ConversionDto {
                kind: "bom",
                lines: 0,
            });
        }
        file.encoding.to = Some(encoding.name());
        file.bom.to = Some(bom);
        file.line_endings.to = Some(doc.document().eol().style());
        file.converted_lines = if changed_encoding {
            physical_lines(&doc)
        } else {
            edited_lines(&doc)
        };
        if file.converted_lines == 0 && !changed_encoding && bom == from_bom {
            return Ok(Ok(None));
        }
        Ok(Ok(Some(Converted { doc, format })))
    }

    /// The question machine of a planned `text convert` with its request's answers (`--yes`, `--force`, `--unmappable
    /// replace`, `--confirm`); `None` when nothing is planned to be written. Standard output asks nothing.
    pub fn text_convert_confirmation(&self, plan: &TextConvertPlan) -> Option<SaveDriver> {
        if plan.target == ConvertTarget::Stdout {
            return None;
        }
        plan.save
            .as_ref()
            .map(|save| self.save_confirmation(save, plan.answers))
    }

    /// The plan as `--dry-run` shows it: nothing is written.
    pub fn text_convert_dry_run(&self, plan: TextConvertPlan) -> TextConvertOutcome {
        let items = plan
            .save
            .as_ref()
            .map(|save| save_plan_items(save, None))
            .unwrap_or_default();
        let mut warnings = plan.warnings.clone();
        if let Some(save) = &plan.save {
            warnings.extend(save.warnings.iter().cloned());
        }
        let plan_lines = plan
            .save
            .as_ref()
            .map(SavePlan::human_lines)
            .unwrap_or_default();
        finish(plan, items, warnings, plan_lines, false)
    }

    /// Executes a planned (and answered) `text convert`: in place and `--output` through the save plan
    /// (`execute_save`); standard output streams the one file's content to `out` (the bytes an in-place save would
    /// write; an item with an open `ChangedOnDisk` fails unless `--force`). A cancel leaves the files not reached
    /// `not_run` (exit 100).
    pub fn text_convert_execute(
        &self,
        plan: TextConvertPlan,
        out: &mut dyn Write,
        job: &JobControl,
    ) -> Result<TextConvertOutcome, ApiError> {
        run_guarded(|| {
            if plan.dry_run {
                return Ok(self.text_convert_dry_run(plan));
            }
            let router = self.router();
            let vfs: &dyn Vfs = &*router;
            let mut warnings = plan.warnings.clone();
            let Some(save) = plan.save.clone() else {
                return Ok(finish(plan, Vec::new(), warnings, Vec::new(), true));
            };
            warnings.extend(save.warnings.iter().cloned());
            let plan_lines = save.human_lines();
            if plan.target == ConvertTarget::Stdout {
                let results = self.stream_stdout(vfs, &plan, &save, out, job)?;
                return Ok(finish_stdout(plan, results, warnings, plan_lines));
            }
            let docs: Vec<&EditableDocument> = plan.docs.iter().collect();
            let report = Self::save_execute_in(vfs, save.clone(), &docs, job);
            warnings.extend(report.op.warnings.iter().cloned());
            let items = save_plan_items(&save, Some(&report.op));
            Ok(finish(plan, items, warnings, plan_lines, true))
        })
    }

    /// Standard output: the content of each planned item streamed from [`SaveSource`] (the bytes an in-place save
    /// writes) to `out`. Returns each item's status and failure.
    fn stream_stdout(
        &self,
        vfs: &dyn Vfs,
        plan: &TextConvertPlan,
        save: &SavePlan,
        out: &mut dyn Write,
        job: &JobControl,
    ) -> Result<Vec<(ItemStatus, Option<ErrorReport>)>, ApiError> {
        let docs: Vec<&EditableDocument> = plan.docs.iter().collect();
        let source = SaveSource::new(vfs, save, &docs);
        let mut results = Vec::with_capacity(save.items.len());
        let mut buffer = vec![0u8; 1 << 16];
        for (item, planned) in save.items.iter().zip(&save.plan.items) {
            if job.cancel.is_cancelled() {
                results.push((ItemStatus::NotRun, None));
                continue;
            }
            if let Some(error) = &planned.error {
                results.push((ItemStatus::Failed, Some(error.clone())));
                continue;
            }
            let target = item.target.canonical();
            if item.questions().contains(&SaveQuestion::ChangedOnDisk) && !plan.answers.force {
                results.push((
                    ItemStatus::Failed,
                    Some(item_error(
                        ErrorKind::InvalidInput,
                        &target,
                        "the file changed on disk since it was loaded: pass --force to convert the text \
                         as loaded"
                            .to_owned(),
                    )),
                ));
                continue;
            }
            let mut reader = match source.open_read(&item.target) {
                Ok(reader) => reader,
                Err(e) => {
                    results.push((ItemStatus::Failed, Some(ErrorReport::capture(&e, None))));
                    continue;
                }
            };
            let status = loop {
                if job.cancel.is_cancelled() {
                    break (ItemStatus::Failed, Some(ApiError::Cancelled.report()));
                }
                match reader.read(&mut buffer) {
                    Ok(0) => break (ItemStatus::Done, None),
                    Ok(n) => {
                        out.write_all(&buffer[..n]).map_err(ApiError::output)?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        break (
                            ItemStatus::Failed,
                            Some(item_error(ErrorKind::Io, &target, e.to_string())),
                        );
                    }
                }
            };
            results.push(status);
        }
        out.flush().map_err(ApiError::output)?;
        Ok(results)
    }

    /// `text convert` in one call (§1.10.23): plan, the request's answers (open questions nobody answers refuse the
    /// request, W6 §1.6.11), then the dry run or the execution; standard output goes to `out`.
    pub fn text_convert(
        &self,
        req: &TextConvertRequest,
        out: &mut dyn Write,
        job: &JobControl,
    ) -> Result<TextConvertOutcome, ApiError> {
        let plan = self.text_convert_plan(req, job)?;
        if plan.dry_run {
            return Ok(self.text_convert_dry_run(plan));
        }
        let plan = match self.text_convert_confirmation(&plan) {
            Some(driver) => {
                if let Some(refusal) = driver.refusal() {
                    return Err(refusal);
                }
                plan.answered(driver)?
            }
            None => plan,
        };
        self.text_convert_execute(plan, out, job)
    }
}

/// The outcome of a plan with its save plan's JSON items (statuses as executed, or as a dry run shows them).
fn finish(
    plan: TextConvertPlan,
    items: Vec<SavePlanItemDto>,
    warnings: Vec<Warning>,
    plan_lines: Vec<String>,
    executed: bool,
) -> TextConvertOutcome {
    let results: Vec<(ItemStatus, Option<SkipReason>, Option<ErrorReport>)> = items
        .iter()
        .map(|item| (item.status, item.reason, item.error.clone()))
        .collect();
    let stdout = plan.target == ConvertTarget::Stdout;
    build(plan, items, results, warnings, plan_lines, executed, stdout)
}

/// The outcome of a standard-output run.
fn finish_stdout(
    plan: TextConvertPlan,
    results: Vec<(ItemStatus, Option<ErrorReport>)>,
    warnings: Vec<Warning>,
    plan_lines: Vec<String>,
) -> TextConvertOutcome {
    let results = results
        .into_iter()
        .map(|(status, error)| (status, None, error))
        .collect();
    build(plan, Vec::new(), results, warnings, plan_lines, true, true)
}

/// The DTO: one file per name, the plan one item per file (standard output: none), the counts.
#[allow(clippy::too_many_arguments)]
fn build(
    plan: TextConvertPlan,
    items: Vec<SavePlanItemDto>,
    results: Vec<(ItemStatus, Option<SkipReason>, Option<ErrorReport>)>,
    warnings: Vec<Warning>,
    plan_lines: Vec<String>,
    executed: bool,
    stdout: bool,
) -> TextConvertOutcome {
    let dry_run = !executed;
    let save_items = plan
        .save
        .as_ref()
        .map(|s| s.items.as_slice())
        .unwrap_or(&[]);
    let mut files = Vec::with_capacity(plan.files.len());
    let mut plan_items = Vec::new();
    let mut summary = ConvertSummaryDto::default();
    for file in plan.files {
        let mut dto = file.dto;
        match file.pre {
            Pre::Failed(error) => {
                dto.status = ItemStatus::Failed;
                if !stdout {
                    plan_items.push(synthetic_item(
                        &dto,
                        ItemStatus::Failed,
                        None,
                        Some(error.clone()),
                    ));
                }
                dto.error = Some(error);
            }
            Pre::NoChange => {
                dto.status = ItemStatus::Skipped;
                dto.skip = Some(SkipReason::NoChange);
                if !stdout {
                    plan_items.push(synthetic_item(
                        &dto,
                        if dry_run {
                            ItemStatus::Planned
                        } else {
                            ItemStatus::Skipped
                        },
                        Some(SkipReason::NoChange),
                        None,
                    ));
                }
            }
            Pre::Planned(i) => {
                let (status, skip, error) =
                    results
                        .get(i)
                        .cloned()
                        .unwrap_or((ItemStatus::NotRun, None, None));
                // A dry run's planned item keeps its skip (declined, duplicate) as skipped.
                dto.status = match (status, skip) {
                    (ItemStatus::Planned, Some(_)) => ItemStatus::Skipped,
                    (status, _) => status,
                };
                dto.skip = skip;
                dto.error = error;
                if let Some(item) = save_items.get(i) {
                    dto.questions = item.questions().iter().map(|q| q.as_str()).collect();
                    let target = item.target.canonical();
                    dto.warnings.extend(
                        warnings
                            .iter()
                            .filter(|w| w.spec.as_deref() == Some(target.as_str()))
                            .filter(|w| !dto.warnings.contains(w))
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                }
                if let Some(item) = items.get(i) {
                    plan_items.push(item.clone());
                }
            }
        }
        summary.files += 1;
        match dto.status {
            ItemStatus::Done | ItemStatus::Planned => {
                summary.done += u64::from(executed);
                summary.converted_lines += dto.converted_lines;
            }
            ItemStatus::Skipped => summary.skipped += 1,
            ItemStatus::Failed => summary.failed += 1,
            ItemStatus::NotRun => summary.not_run += 1,
        }
        files.push(dto);
    }
    let ok = summary.failed == 0 && summary.not_run == 0;
    TextConvertOutcome {
        dto: TextConvertDto {
            dry_run,
            target: plan.target.as_str(),
            files,
            plan: plan_items,
            summary,
        },
        warnings,
        exit_code: if ok {
            ExitCode::Success
        } else {
            ExitCode::UnknownError
        },
        plan_lines,
    }
}

/// The plan item of a file that is not in the save plan (refused before planning, or `no_change`).
fn synthetic_item(
    file: &ConvertFileDto,
    status: ItemStatus,
    reason: Option<SkipReason>,
    error: Option<ErrorReport>,
) -> SavePlanItemDto {
    SavePlanItemDto {
        action: if reason.is_some() { "skip" } else { "write" },
        kind: hikasa_vfs::EntryKind::File,
        source: Some(file.spec.clone()),
        target: file.spec.clone(),
        size: None,
        replaces: false,
        backup: None,
        expect: None,
        status,
        reason,
        error,
        via_link: None,
        encoding: file.encoding.to.or(file.encoding.from).unwrap_or(""),
        bom: file.bom.to.or(file.bom.from).unwrap_or(false),
        bytes: 0,
        spliced_bytes: 0,
        encoded_lines: 0,
    }
}
