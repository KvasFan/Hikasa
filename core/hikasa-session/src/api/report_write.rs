//! Writing reports (M2 W7, §1.7.8, §1.7.9): the names of a linked report's files ([`safe_component`] and the rest,
//! pure), the plan of a report ([`ReportPlan`], [`Service::report_plan`]) and its execution
//! ([`Service::report_execute`]): a file target streamed through the atomic writer, the clipboard payload, the printer
//! and browser files below `<cache>/print/`, and a folder report with its pair reports in a files folder that is
//! written beside, then swapped in. No `std::fs` here (architecture check 2): every read and write goes through the
//! VFS.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::PathBuf;

use hikasa_core::{
    CancelToken, Classify, ErrorReport, FileTime, Phase, Side, Spec, Warning, WarningCode,
};
use hikasa_folder::{ByteBudget, NodeId};
use hikasa_report::{
    FolderLink, HtmlStyle, OutputFormat, PairLink, PairLinks, ReportBody, ReportError, ReportInput,
    ReportKind, ReportMeta, ReportRequest, ReportScope, ReportWords, href, linked_rows,
    render_report,
};
use hikasa_text::DEFAULT_RULES_MAX_BYTES;
use hikasa_vfs::archive::name::windows_name_problem;
use hikasa_vfs::ops::ItemStamp;
use hikasa_vfs::{
    Capabilities, Durability, EntryKind, FileName, FollowLinks, ListOptions, Vfs, VfsError,
    WriteOptions,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::folder_ops::SelectRequest;
use super::report::{
    REPORT_BUFFER, ReportSource, ReportTarget, ReportVerdict, TargetDto, render_error, write_error,
};
use super::{ApiError, Service};
use crate::clipboard::{CLIPBOARD_REPORT_MAX, ClipboardPayload};
use crate::jobs::{JobControl, run_guarded};

/// The name of a linked report's files folder's marker.
pub const MARKER: &str = ".hikasa-report.toml";

/// A name that is safe on every file system and in a link (normative, §1.7.8): every char that is not a letter or a
/// digit (Unicode) or one of `.`, `-`, `_` becomes `_` (a lossy U+FFFD too); a leading `.` or `-` becomes `_`; the name
/// is cut at a char boundary to `max` bytes; a name Windows refuses (a device name, also before an extension) gets a
/// leading `_`, and a trailing `.` becomes `_` (W7a note: a leading `_` alone leaves a trailing dot); empty → `file`.
/// The result passes [`windows_name_problem`], and its ASCII bytes are all RFC 3986 unreserved, so an href keeps
/// everything but its non-ASCII letters.
pub fn safe_component(name: &str, max: usize) -> String {
    let max = max.max(1);
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    truncate(&mut out, max);
    if out.starts_with(['.', '-']) {
        out.replace_range(..1, "_");
    }
    if out.ends_with('.') {
        out.pop();
        out.push('_');
    }
    if windows_name_problem(&out).is_some() {
        out.insert(0, '_');
        truncate(&mut out, max);
        if out.ends_with('.') {
            out.pop();
            out.push('_');
        }
    }
    if out.is_empty() {
        return "file".to_owned();
    }
    out
}

/// Cuts `s` to at most `max` bytes at a char boundary.
fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut at = max;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        s.truncate(at);
    }
}

/// The stem of the main file's final name: without its last extension; `report` when that leaves nothing
/// (`report.html` → `report`, `.html` → `report`).
pub fn main_stem(final_name: &OsStr) -> String {
    let name = final_name.to_string_lossy();
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_ref(), |(stem, _)| stem);
    if stem.is_empty() {
        "report".to_owned()
    } else {
        stem.to_owned()
    }
}

/// The files folder wanted next to the main file: `<safe_component(stem, 64)>_files`; `-2`, `-3`, … when that one is
/// not this report's (`attempt` 1 is the plain name).
pub fn files_folder_name(final_name: &OsStr, attempt: u32) -> String {
    let base = format!("{}_files", safe_component(&main_stem(final_name), 64));
    if attempt <= 1 {
        base
    } else {
        format!("{base}-{attempt}")
    }
}

/// The file of pair report `n` (1-based) of `total`: `<n>-<safe_component(left final name, 64)>.html`, `n` zero-padded
/// to the digits of `total`. The numbers make the names distinct on every file system, case-insensitive ones too.
pub fn pair_file_name(n: usize, total: usize, left_name: &OsStr) -> String {
    let width = total.max(1).checked_ilog10().map_or(1, |d| d as usize + 1);
    format!(
        "{n:0width$}-{}.html",
        safe_component(&left_name.to_string_lossy(), 64)
    )
}

// ---- plans ----------------------------------------------------------------------------------------------------

/// The last `_files-<n>` tried before `already_exists` (§1.7.9 step 3).
const MAX_FOLDER_ATTEMPT: u32 = 99;

/// How long printer and browser folders below `<cache>/print/` are kept (§1.7.11).
const PRINT_KEEP_DAYS: i64 = 7;

/// The plan of one report (§1.7.9): the target and, for a linked report, its files folder and pairs. A dry run's plan
/// is the plan execution takes (T12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportPlan {
    /// The kind of the report.
    pub kind: ReportKind,
    /// The request's canonical words.
    pub words: ReportWords,
    /// The target.
    pub target: TargetDto,
    /// The file written (File, Printer, Browser).
    pub main: Option<Spec>,
    /// The main file exists.
    pub replaces: bool,
    /// The files folder and the pair reports of `include-file-links`.
    pub links: Option<LinksPlan>,
    /// The warnings of the plan (`report_option_unused`, `report_folder_renamed`).
    pub warnings: Vec<Warning>,
    /// The canonical layout word (additive, W7b: `hikasa.report.v1`'s `layout`).
    pub layout: String,
    /// `text`, `html`, `xml`, `csv` or `json` (additive, W7b: `hikasa.report.v1`'s `format`).
    pub format: String,
    /// The words the layout does not use (additive, W7b: `hikasa.report.v1`'s `unused`).
    pub unused: Vec<String>,
    /// The folder below `<cache>/print/` a printer or browser report is written into (additive, W7b).
    pub print_folder: Option<Spec>,
    /// The report time, taken once per job (D-206); a deserialised plan takes it again.
    #[serde(skip)]
    pub(crate) time: Option<jiff::Zoned>,
    /// The stylesheet of `html-custom:<css>`, read by the plan.
    #[serde(skip)]
    pub(crate) css: Option<String>,
}

/// The files folder of a linked report (§1.7.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinksPlan {
    /// The folder the pair reports go to.
    pub folder: Spec,
    /// The folder wanted, `<safe stem>_files`; `folder != wanted`: renamed, warning `report_folder_renamed`.
    pub wanted: Spec,
    /// New, or an earlier report's to replace.
    pub state: FilesFolder,
    /// The pairs, in report order.
    pub pairs: Vec<PairPlan>,
    /// The old folder's listing (`ReplaceOurs`), checked again at execution: name and stamp.
    pub stamps: Vec<(String, ItemStamp)>,
}

/// What the files folder is before the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesFolder {
    /// Absent: made.
    New,
    /// An earlier report's (its marker names this report): replaced.
    ReplaceOurs,
}

/// One pair report of a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairPlan {
    /// The row.
    pub node: NodeId,
    /// The row's path (D-29).
    pub path: String,
    /// The pair report's file name in the files folder.
    pub file: String,
    /// The left file.
    pub left: Spec,
    /// The right file.
    pub right: Spec,
    /// Known to be skipped before anything is read (`too_large`).
    pub skip: Option<String>,
}

/// What became of a pair report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairStatus {
    /// A dry run's pair.
    Planned,
    /// Written.
    Written,
    /// Skipped (`too_large`, `binary`).
    Skipped,
    /// Its comparison or rendering failed.
    Failed,
}

/// The outcome of one pair report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairOutcome {
    /// The row's path.
    pub path: String,
    /// The file name, when written or planned.
    pub file: Option<String>,
    /// What became of it.
    pub status: PairStatus,
    /// Why it was skipped.
    pub reason: Option<String>,
    /// Why it failed.
    pub error: Option<ErrorReport>,
}

/// The outcome of a report (§1.7.9).
#[derive(Debug, Clone)]
pub struct ReportOutcome {
    /// The plan that ran.
    pub plan: ReportPlan,
    /// Nothing was written.
    pub dry_run: bool,
    /// The job was cancelled after the report could no longer be stopped (step 5 on): the report stands.
    pub cancelled: bool,
    /// The bytes of the main report.
    pub bytes: u64,
    /// The pair reports, in report order.
    pub pairs: Vec<PairOutcome>,
    /// The payload for the clipboard target; the adapter places it.
    pub clipboard: Option<ClipboardPayload>,
    /// The file handed to the associated application (printer, browser).
    pub opened: Option<PathBuf>,
    /// The plan's and the execution's warnings, then the rendering's.
    pub warnings: Vec<Warning>,
    /// The comparison's verdict, as `hikasa.text_compare.v1` and `hikasa.folder_compare.v1` write it (additive, W7b).
    pub verdict: ReportVerdict,
}

/// `hikasa.report.v1`'s `result` (§1.7.15).
#[derive(Debug, Clone, Serialize)]
pub struct ReportDto {
    /// `text` or `folder`.
    #[serde(rename = "type")]
    pub kind: ReportKind,
    /// The canonical layout word.
    pub layout: String,
    /// `text`, `html`, `xml`, `csv` or `json`.
    pub format: String,
    /// `file`, `clipboard`, `printer` or `browser`.
    pub target: TargetDto,
    /// The main file.
    pub output: Option<String>,
    /// The main file existed.
    pub replaced: bool,
    /// A dry run.
    pub dry_run: bool,
    /// Cancelled once the report could no longer be stopped.
    pub cancelled: bool,
    /// Bytes of the main report.
    pub bytes: u64,
    /// The comparison's verdict.
    pub verdict: ReportVerdict,
    /// The canonical words.
    pub words: WordsDto,
    /// The words the layout does not use.
    pub unused: Vec<String>,
    /// The files folder of a linked report.
    pub links: Option<LinksDto>,
    /// The sizes of the clipboard payload.
    pub clipboard: Option<ReportClipboardDto>,
    /// The file handed to the associated application.
    pub opened: Option<String>,
}

/// The words of `hikasa.report.v1`.
#[derive(Debug, Clone, Serialize)]
pub struct WordsDto {
    /// The layout word.
    pub layout: Option<String>,
    /// The option words.
    pub options: Vec<String>,
    /// The output words.
    pub output_options: Vec<String>,
}

/// The links of `hikasa.report.v1`.
#[derive(Debug, Clone, Serialize)]
pub struct LinksDto {
    /// The files folder.
    pub folder: String,
    /// The folder wanted.
    pub wanted: String,
    /// `folder != wanted`.
    pub renamed: bool,
    /// `new` or `replace_ours`.
    pub state: FilesFolder,
    /// The pairs.
    pub pairs: Vec<PairOutcome>,
    /// The counts of the pairs.
    pub counts: PairCounts,
}

/// Written, skipped and failed pairs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PairCounts {
    /// Written (planned, in a dry run).
    pub written: usize,
    /// Skipped.
    pub skipped: usize,
    /// Failed.
    pub failed: usize,
}

/// The sizes of a clipboard payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReportClipboardDto {
    /// Bytes of the text.
    pub text_bytes: u64,
    /// Bytes of the HTML.
    pub html_bytes: Option<u64>,
}

impl ReportOutcome {
    /// `hikasa.report.v1`'s `result`.
    pub fn dto(&self) -> ReportDto {
        let plan = &self.plan;
        ReportDto {
            kind: plan.kind,
            layout: plan.layout.clone(),
            format: plan.format.clone(),
            target: plan.target,
            output: plan.main.as_ref().map(Spec::canonical),
            replaced: plan.replaces,
            dry_run: self.dry_run,
            cancelled: self.cancelled,
            bytes: self.bytes,
            verdict: self.verdict.clone(),
            words: WordsDto {
                layout: plan.words.layout.clone(),
                options: plan.words.options.clone(),
                output_options: plan.words.output_options.clone(),
            },
            unused: plan.unused.clone(),
            links: plan.links.as_ref().map(|links| {
                let mut counts = PairCounts::default();
                for pair in &self.pairs {
                    match pair.status {
                        PairStatus::Planned | PairStatus::Written => counts.written += 1,
                        PairStatus::Skipped => counts.skipped += 1,
                        PairStatus::Failed => counts.failed += 1,
                    }
                }
                LinksDto {
                    folder: links.folder.canonical(),
                    wanted: links.wanted.canonical(),
                    renamed: links.folder != links.wanted,
                    state: links.state,
                    pairs: self.pairs.clone(),
                    counts,
                }
            }),
            clipboard: self.clipboard.as_ref().map(|p| ReportClipboardDto {
                text_bytes: p.text.len() as u64,
                html_bytes: p.html.as_ref().map(|h| h.len() as u64),
            }),
            opened: self.opened.as_ref().map(|p| p.display().to_string()),
        }
    }

    /// The human lines of a dry run (§1.7.9): `write <main>` or `replace <main>`; with links `mkdir <folder>`,
    /// `replace <folder> (an earlier report)` or `mkdir <folder> (instead of <wanted>, which is not this report's)`,
    /// then `write <folder>/<file>  <path>` per pair (`skip (<reason>) <path>` for one known to be skipped), then
    /// `would write <n> files`.
    pub fn dry_run_lines(&self) -> Vec<String> {
        let plan = &self.plan;
        let mut lines = Vec::new();
        let mut files = 0usize;
        if let Some(folder) = &plan.print_folder {
            lines.push(format!("mkdir {}", folder.canonical()));
        }
        match (&plan.main, plan.target) {
            (Some(main), _) => {
                let verb = if plan.replaces { "replace" } else { "write" };
                lines.push(format!("{verb} {}", main.canonical()));
                files += 1;
            }
            (None, TargetDto::Clipboard) => lines.push("copy to the clipboard".to_owned()),
            (None, _) => lines.push("write to standard output".to_owned()),
        }
        if let Some(links) = &plan.links {
            let folder = links.folder.canonical();
            lines.push(match (links.state, links.folder == links.wanted) {
                (FilesFolder::ReplaceOurs, _) => format!("replace {folder} (an earlier report)"),
                (FilesFolder::New, true) => format!("mkdir {folder}"),
                (FilesFolder::New, false) => format!(
                    "mkdir {folder} (instead of {}, which is not this report's)",
                    links.wanted.canonical()
                ),
            });
            for pair in &links.pairs {
                match &pair.skip {
                    Some(reason) => lines.push(format!("skip ({reason}) {}", pair.path)),
                    None => {
                        lines.push(format!("write {folder}/{}  {}", pair.file, pair.path));
                        files += 1;
                    }
                }
            }
            files += 1; // the marker
        }
        lines.push(format!(
            "would write {files} {}",
            if files == 1 { "file" } else { "files" }
        ));
        lines
    }
}

/// The marker of a files folder (§1.7.9, T13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    /// 1.
    schema_version: u32,
    /// The main file's final name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report: Option<String>,
    /// The main file's final name as hex of its OS bytes, when it is not UTF-8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report_raw: Option<String>,
    /// `hikasa <version>`.
    generator: String,
    /// The pair reports.
    #[serde(default)]
    file: Vec<MarkerFile>,
}

/// One file of a marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MarkerFile {
    /// The file name.
    name: String,
    /// Its size.
    size: u64,
    /// Its modification time, nanoseconds since the epoch, as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modified: Option<i64>,
}

/// Nanoseconds since the epoch (saturating).
fn nanos(time: FileTime) -> i64 {
    i64::try_from(i128::from(time.unix_seconds()) * 1_000_000_000 + i128::from(time.subsec_nanos()))
        .unwrap_or(i64::MAX)
}

/// Lower-case hex of bytes.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes of an OS name: the OS bytes on Unix, the WTF-8 bytes elsewhere.
fn os_bytes(name: &OsStr) -> Vec<u8> {
    name.as_encoded_bytes().to_vec()
}

/// The final name of a spec: the OS name of a local path, the last segment otherwise.
fn final_name(spec: &Spec) -> Option<OsString> {
    match spec {
        Spec::Local(path) => path.file_name().map(OsStr::to_os_string),
        other => other.file_name().map(OsString::from),
    }
}

/// The local path of a spec, when it is one.
fn local_path(spec: &Spec) -> Option<PathBuf> {
    match spec {
        Spec::Local(path) => Some(path.as_path().to_path_buf()),
        _ => None,
    }
}

/// The plan's answers to the renderer: which row links where.
struct PlanLinks {
    links: BTreeMap<NodeId, PairLink>,
}

impl PairLinks for PlanLinks {
    fn link(&self, node: NodeId) -> Option<PairLink> {
        self.links.get(&node).cloned()
    }
}

/// A writer into memory that refuses to grow beyond `limit` bytes (the clipboard's cap).
struct CapWriter {
    buf: Vec<u8>,
    limit: u64,
    over: bool,
}

impl Write for CapWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() as u64 + data.len() as u64 > self.limit {
            self.over = true;
            return Err(std::io::Error::other("the clipboard limit was reached"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The `format` word of a request.
fn format_word(req: &ReportRequest) -> &'static str {
    match req.layout_word() {
        "xml" => "xml",
        "statistics" => "csv",
        "json" => "json",
        _ => match req.format() {
            OutputFormat::Html { .. } => "html",
            OutputFormat::Text { .. } => "text",
        },
    }
}

/// `req` with the comparison's effective ignore-unimportant (M1's rule, wave-4 review): the report, the verdict and
/// the exit code agree.
fn effective(req: &ReportRequest, source: &ReportSource<'_>) -> ReportRequest {
    let mut req = req.clone();
    match (&mut req.body, source) {
        (ReportBody::Text(opts), ReportSource::Text(resp)) => {
            opts.ignore_unimportant = resp.ignore_unimportant;
        }
        (ReportBody::Folder { options, links }, ReportSource::Folder { response, .. }) => {
            options.display.ignore_unimportant = response.ignore_unimportant;
            if let Some(links) = links {
                links.text.ignore_unimportant = response.ignore_unimportant;
            }
        }
        _ => {}
    }
    req
}

/// The time of a print folder's name, `<yyyymmdd>-<hhmmss>` (then `-<n>`), as a civil time; `None` for another name.
fn print_folder_time(name: &str) -> Option<jiff::civil::DateTime> {
    let mut parts = name.split('-');
    let (date, time, n) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || date.len() != 8
        || time.len() != 6
        || n.is_empty()
        || ![date, time, n]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let num = |s: &str| s.parse::<i16>().ok();
    let (y, mo, d) = (
        num(date.get(..4)?)?,
        num(date.get(4..6)?)?,
        num(date.get(6..)?)?,
    );
    let (h, mi, sec) = (
        num(time.get(..2)?)?,
        num(time.get(2..4)?)?,
        num(time.get(4..)?)?,
    );
    jiff::civil::DateTime::new(
        y,
        i8::try_from(mo).ok()?,
        i8::try_from(d).ok()?,
        i8::try_from(h).ok()?,
        i8::try_from(mi).ok()?,
        i8::try_from(sec).ok()?,
        0,
    )
    .ok()
}

impl Service {
    /// The child `name` of the folder `dir`.
    fn child_of(&self, vfs: &dyn Vfs, dir: &Spec, name: &OsStr) -> Result<Spec, ApiError> {
        vfs.child(dir, &FileName::from_os(name))
            .map_err(ApiError::from_vfs)
    }

    /// True when the back-end at `at` tells names apart by case.
    fn case_sensitive(&self, vfs: &dyn Vfs, at: &Spec) -> bool {
        vfs.capabilities(at)
            .is_ok_and(|caps| caps.contains(Capabilities::CASE_SENSITIVE))
    }

    /// The listing of a folder: name → stamp; `None` when it cannot be listed.
    fn folder_stamps(&self, vfs: &dyn Vfs, dir: &Spec) -> Option<Vec<(String, ItemStamp)>> {
        let options = ListOptions {
            follow: FollowLinks::No,
            prefilter: None,
        };
        let listing = vfs.list(dir, &options, &CancelToken::new()).ok()?;
        if !listing.errors.is_empty() || !listing.name_only.is_empty() {
            return None;
        }
        let mut stamps: Vec<(String, ItemStamp)> = listing
            .entries
            .iter()
            .map(|entry| {
                let file = entry.kind == EntryKind::File;
                (
                    entry.name.as_str().to_owned(),
                    ItemStamp {
                        kind: entry.kind,
                        size: file.then_some(entry.size),
                        mtime: if file { entry.mtime } else { None },
                    },
                )
            })
            .collect();
        if listing.entries.iter().any(|e| e.name.is_lossy()) {
            return None;
        }
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        Some(stamps)
    }

    /// Whether `dir` is a files folder of the main file `main_name` (§1.7.9 step 2): a folder holding a marker that
    /// parses, names this report (as the back-end compares names) and lists exactly the other files, each with its
    /// size and modification time. `Some(stamps)` when it is.
    fn ours(
        &self,
        vfs: &dyn Vfs,
        dir: &Spec,
        main_name: &OsStr,
    ) -> Option<Vec<(String, ItemStamp)>> {
        let stamps = self.folder_stamps(vfs, dir)?;
        let marker_spec = self.child_of(vfs, dir, OsStr::new(MARKER)).ok()?;
        let mut text = String::new();
        let mut reader = vfs.open_read(&marker_spec).ok()?;
        std::io::Read::read_to_string(&mut std::io::Read::take(&mut reader, 1 << 20), &mut text)
            .ok()?;
        let marker: Marker = toml::from_str(&text).ok()?;
        if marker.schema_version != 1 {
            return None;
        }
        let named = match (&marker.report, &marker.report_raw, main_name.to_str()) {
            (Some(report), _, Some(main)) => {
                if self.case_sensitive(vfs, dir) {
                    report == main
                } else {
                    report.to_lowercase() == main.to_lowercase()
                }
            }
            (_, Some(raw), _) => *raw == hex(&os_bytes(main_name)),
            _ => false,
        };
        if !named {
            return None;
        }
        // Exactly the marker and the listed files, each as listed.
        let mut want: BTreeMap<&str, &MarkerFile> =
            marker.file.iter().map(|f| (f.name.as_str(), f)).collect();
        for (name, stamp) in &stamps {
            if name == MARKER {
                if stamp.kind != EntryKind::File {
                    return None;
                }
                continue;
            }
            let file = want.remove(name.as_str())?;
            if stamp.kind != EntryKind::File
                || stamp.size != Some(file.size)
                || stamp.mtime.map(nanos) != file.modified
            {
                return None;
            }
        }
        if !want.is_empty() || !stamps.iter().any(|(n, _)| n == MARKER) {
            return None;
        }
        Some(stamps)
    }

    /// The plan of a report (§1.7.9). Sets the scope of `--select`/`--path` (`select`; W7b: `ReportFlags::select`
    /// reaches the plan here, after the comparison) and reads the stylesheet of `html-custom:<css>` through the VFS (at
    /// most 1 MiB, `too_large`; `-` and `clipboard://` refused). Without links a plan is the target: `main` through
    /// `write_spec` (an archive member allowed, its folder must exist), `replaces` from one `lstat`. With links, the
    /// files folder: `<safe stem>_files` when absent or this report's, else the first `_files-<n>` that is, with the
    /// warning `report_folder_renamed`; none of 99 → `already_exists`.
    pub fn report_plan(
        &self,
        source: ReportSource<'_>,
        req: &mut ReportRequest,
        target: &ReportTarget,
        select: Option<&SelectRequest>,
        job: &JobControl,
    ) -> Result<ReportPlan, ApiError> {
        self.report_plan_on(&*self.router(), source, req, target, select, job)
    }

    /// [`Service::report_plan`] with the back-end the report is written to (additive, W7b): the Service's router for
    /// the adapters; a test's `MemoryVfs` with its faults and journal. The comparisons, the stylesheet and the specs of
    /// the pairs are read as always.
    pub fn report_plan_on(
        &self,
        vfs: &dyn Vfs,
        source: ReportSource<'_>,
        req: &mut ReportRequest,
        target: &ReportTarget,
        select: Option<&SelectRequest>,
        job: &JobControl,
    ) -> Result<ReportPlan, ApiError> {
        run_guarded(|| {
            if let (Some(select), ReportSource::Folder { prepared, .. }) = (select, &source) {
                let read = prepared.model().read();
                let (selection, _) =
                    self.select_of(&read, select, hikasa_folder::select::SideSet::all())?;
                req.scope = ReportScope::FolderRows(selection);
            }
            let css = match &req.css {
                Some(spec) => Some(self.read_stylesheet(spec)?),
                None => None,
            };
            let time = self.report_time();
            let mut warnings: Vec<Warning> = req.unused_warning().into_iter().collect();
            let (main, print_folder) = match target {
                ReportTarget::Stdout | ReportTarget::Clipboard => (None, None),
                ReportTarget::File(raw) => (Some(self.target_spec(vfs, raw)?), None),
                ReportTarget::Printer | ReportTarget::Browser => {
                    let folder = self.print_folder(vfs, time.as_ref())?;
                    let title = req.title.as_deref().unwrap_or("report");
                    let name = format!("{}.html", safe_component(title, 64));
                    let main = self.child_of(vfs, &folder, OsStr::new(&name))?;
                    (Some(main), Some(folder))
                }
            };
            let replaces = match (&main, &print_folder) {
                (Some(main), None) => vfs.stat(main, FollowLinks::No).is_ok(),
                _ => false,
            };
            let links = match (&req.body, &main, &source) {
                (
                    ReportBody::Folder {
                        links: Some(_),
                        options,
                    },
                    Some(main),
                    ReportSource::Folder { response, .. },
                ) => {
                    if !matches!(main, Spec::Local(_)) {
                        return Err(ApiError::Report(ReportError::Refused {
                            message: "file reports need a folder on disk",
                        }));
                    }
                    let mut options = *options;
                    options.display.ignore_unimportant = response.ignore_unimportant;
                    let scope = match &req.scope {
                        ReportScope::FolderRows(selection) => Some(selection),
                        _ => None,
                    };
                    let rows = linked_rows(&response.comparison, &options, scope);
                    let plan = self.links_plan(
                        vfs,
                        main,
                        &response.comparison,
                        &rows,
                        print_folder.is_some(),
                        job,
                    )?;
                    if plan.folder != plan.wanted {
                        warnings.push(Warning {
                            code: WarningCode::ReportFolderRenamed,
                            message: format!(
                                "{} is not this report's, so the pair reports go to {}",
                                plan.wanted.canonical(),
                                plan.folder.canonical()
                            ),
                            spec: None,
                        });
                    }
                    Some(plan)
                }
                _ => None,
            };
            Ok(ReportPlan {
                kind: req.kind,
                words: req.words(),
                target: target.dto(),
                main,
                replaces,
                links,
                warnings,
                layout: req.layout_word().to_owned(),
                format: format_word(req).to_owned(),
                unused: req.unused.clone(),
                print_folder,
                time,
                css,
            })
        })
    }

    /// The spec of a file target on `vfs` (as `--output`: an archive member is a member, its folder must exist).
    fn target_spec(&self, vfs: &dyn Vfs, raw: &OsStr) -> Result<Spec, ApiError> {
        let spec = Spec::parse_os(raw, &self.spec_context())?;
        hikasa_vfs::resolve_spec(vfs, spec, self.use_of(raw), &CancelToken::never())
            .map_err(|_| ApiError::Cancelled)
    }

    /// Reads the stylesheet of `html-custom:<css>` (§1.7.14): through the VFS, at most 1 MiB.
    fn read_stylesheet(&self, raw: &str) -> Result<String, ApiError> {
        const MAX: u64 = 1 << 20;
        if raw == "-" || raw.eq_ignore_ascii_case("clipboard://") {
            return Err(ApiError::InvalidArgument(format!(
                "html-custom:{raw}: a stylesheet is read from a file, not from standard input or the clipboard"
            )));
        }
        let spec = self.parse_spec(OsStr::new(raw))?;
        let mut reader = self.router().open_read(&spec).map_err(ApiError::from_vfs)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(&mut reader, MAX + 1), &mut bytes)
            .map_err(|e| {
                ApiError::from_vfs(VfsError::from_io(
                    spec.canonical(),
                    hikasa_vfs::VfsOp::Read,
                    e,
                ))
            })?;
        if bytes.len() as u64 > MAX {
            return Err(ApiError::Report(ReportError::TooLarge { limit: MAX }));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A free folder `<cache>/print/<yyyymmdd>-<hhmmss>-<n>` (§1.7.11), not made yet.
    fn print_folder(&self, vfs: &dyn Vfs, time: Option<&jiff::Zoned>) -> Result<Spec, ApiError> {
        let at = time.cloned().unwrap_or_else(|| self.clock().now());
        let stamp = at.strftime("%Y%m%d-%H%M%S").to_string();
        let root = self.print_root()?;
        for n in 1..=9999u32 {
            let name = format!("{stamp}-{n}");
            let spec = self.child_of(vfs, &root, OsStr::new(&name))?;
            match vfs.stat(&spec, FollowLinks::No) {
                Err(e) if e.kind() == hikasa_core::ErrorKind::NotFound => return Ok(spec),
                Err(e) => return Err(ApiError::from_vfs(e)),
                Ok(_) => {}
            }
        }
        Err(ApiError::AlreadyExists(format!(
            "every print folder of {stamp} below {} is taken",
            root.canonical()
        )))
    }

    /// `<cache>/print`.
    fn print_root(&self) -> Result<Spec, ApiError> {
        let path = self.config().cache_dir.join("print");
        self.parse_spec(path.as_os_str())
    }

    /// The files folder of a linked report (§1.7.9 steps 1–3) and its pairs.
    fn links_plan(
        &self,
        vfs: &dyn Vfs,
        main: &Spec,
        cmp: &hikasa_folder::FolderComparison,
        rows: &[NodeId],
        fresh: bool,
        job: &JobControl,
    ) -> Result<LinksPlan, ApiError> {
        let main_name = final_name(main)
            .ok_or_else(|| ApiError::InvalidArgument("the report needs a file name".into()))?;
        let dir = main
            .parent()
            .ok_or_else(|| ApiError::InvalidArgument("the report needs a folder".into()))?;
        // The main file's folder must exist (as `--output`'s), before any name below it is tried.
        match vfs.stat(&dir, FollowLinks::Yes) {
            Ok(entry) if entry.kind == EntryKind::Dir => {}
            Ok(_) => {
                return Err(ApiError::from_vfs(VfsError::from_io(
                    dir.canonical(),
                    hikasa_vfs::VfsOp::Stat,
                    std::io::Error::from(std::io::ErrorKind::NotADirectory),
                )));
            }
            Err(e) => return Err(ApiError::from_vfs(e)),
        }
        let read = cmp.read();
        let total = rows.len();
        let mut pairs = Vec::with_capacity(total);
        for (i, &node) in rows.iter().enumerate() {
            let (Some(view), Some(left), Some(right)) = (
                read.node(node),
                read.spec(node, Side::Left),
                read.spec(node, Side::Right),
            ) else {
                continue;
            };
            let left_name = view.left.as_ref().map_or("file", |e| e.name);
            let too_large = [view.left.as_ref(), view.right.as_ref()]
                .into_iter()
                .flatten()
                .any(|e| e.size > DEFAULT_RULES_MAX_BYTES);
            pairs.push(PairPlan {
                node,
                path: read.path(node).as_str().to_owned(),
                file: pair_file_name(i + 1, total, OsStr::new(left_name)),
                left,
                right,
                skip: too_large.then(|| "too_large".to_owned()),
            });
        }
        let wanted = self.child_of(vfs, &dir, OsStr::new(&files_folder_name(&main_name, 1)))?;
        for attempt in 1..=MAX_FOLDER_ATTEMPT {
            job.cancel.check().map_err(|_| ApiError::Cancelled)?;
            let folder = if attempt == 1 {
                wanted.clone()
            } else {
                self.child_of(
                    vfs,
                    &dir,
                    OsStr::new(&files_folder_name(&main_name, attempt)),
                )?
            };
            let state = match vfs.stat(&folder, FollowLinks::No) {
                Err(e) if e.kind() == hikasa_core::ErrorKind::NotFound => {
                    Some((FilesFolder::New, Vec::new()))
                }
                Err(_) => None,
                Ok(entry) if entry.kind == EntryKind::Dir && !fresh => self
                    .ours(vfs, &folder, &main_name)
                    .map(|stamps| (FilesFolder::ReplaceOurs, stamps)),
                Ok(_) => None,
            };
            if let Some((state, stamps)) = state {
                return Ok(LinksPlan {
                    folder,
                    wanted,
                    state,
                    pairs,
                    stamps,
                });
            }
        }
        Err(ApiError::AlreadyExists(format!(
            "{} and {}-2 … -{MAX_FOLDER_ATTEMPT} are not this report's: move one away",
            wanted.canonical(),
            wanted.canonical(),
        )))
    }

    /// A dry run (§1.7.9): the plan, nothing written; pairs `planned`, or `skipped` when known.
    pub fn report_dry_run(&self, plan: ReportPlan) -> ReportOutcome {
        let pairs = plan
            .links
            .as_ref()
            .map(|links| {
                links
                    .pairs
                    .iter()
                    .map(|pair| PairOutcome {
                        path: pair.path.clone(),
                        file: pair.skip.is_none().then(|| pair.file.clone()),
                        status: if pair.skip.is_some() {
                            PairStatus::Skipped
                        } else {
                            PairStatus::Planned
                        },
                        reason: pair.skip.clone(),
                        error: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let warnings = plan.warnings.clone();
        ReportOutcome {
            plan,
            dry_run: true,
            cancelled: false,
            bytes: 0,
            pairs,
            clipboard: None,
            opened: None,
            warnings,
            verdict: ReportVerdict::None,
        }
    }

    /// The header and style of a report of `source`.
    fn meta_and_style(
        &self,
        source: &ReportSource<'_>,
        req: &ReportRequest,
        plan: &ReportPlan,
        time: Option<jiff::Zoned>,
    ) -> (ReportMeta, HtmlStyle) {
        let mut meta = match (source, &req.body) {
            (ReportSource::Text(resp), ReportBody::Text(opts)) => {
                ReportMeta::for_text(&resp.comparison, opts, time, &self.generator())
            }
            (ReportSource::Folder { response, .. }, ReportBody::Folder { options, .. }) => {
                ReportMeta::for_folder(&response.comparison, options, time, &self.generator())
            }
            // A kind that does not match is refused by `render_report`.
            _ => ReportMeta {
                title: None,
                generated_at: time,
                generator: self.generator(),
                mode: None,
                filters: None,
                errors: hikasa_core::Pair::new(None, None),
                folder_link: None,
            },
        };
        if let Some(title) = &req.title {
            meta = meta.with_title(title.clone());
        }
        let style = HtmlStyle {
            custom_css: plan.css.clone(),
            ..HtmlStyle::default()
        };
        (meta, style)
    }

    /// Runs a plan (§1.7.9): renders the report of `source` into its target; a linked report writes its pair reports
    /// into a staging folder first and swaps it in (the execution table's order). `stdout` is the writer of the
    /// stdout target. The report time is the plan's: the folder report and its pair reports carry one instant.
    pub fn report_execute(
        &self,
        source: ReportSource<'_>,
        req: &ReportRequest,
        plan: ReportPlan,
        stdout: Option<&mut dyn Write>,
        job: &JobControl,
    ) -> Result<ReportOutcome, ApiError> {
        self.report_execute_on(&*self.router(), source, req, plan, stdout, job)
    }

    /// [`Service::report_execute`] with the back-end the report is written to (additive, W7b; see
    /// [`Service::report_plan_on`]).
    pub fn report_execute_on(
        &self,
        vfs: &dyn Vfs,
        source: ReportSource<'_>,
        req: &ReportRequest,
        plan: ReportPlan,
        stdout: Option<&mut dyn Write>,
        job: &JobControl,
    ) -> Result<ReportOutcome, ApiError> {
        run_guarded(|| {
            let mut plan = plan;
            if plan.time.is_none() {
                plan.time = self.report_time();
            }
            if plan.css.is_none()
                && let Some(spec) = &req.css
            {
                plan.css = Some(self.read_stylesheet(spec)?);
            }
            let req = effective(req, &source);
            let (meta, style) = self.meta_and_style(&source, &req, &plan, plan.time.clone());
            let input = match &source {
                ReportSource::Text(resp) => ReportInput::Text(&resp.comparison),
                ReportSource::Folder { response, .. } => ReportInput::Folder(&response.comparison),
            };
            let mut outcome = ReportOutcome {
                warnings: plan.warnings.clone(),
                plan: plan.clone(),
                dry_run: false,
                cancelled: false,
                bytes: 0,
                pairs: Vec::new(),
                clipboard: None,
                opened: None,
                verdict: source.verdict(),
            };
            job.cancel.check().map_err(|_| ApiError::Cancelled)?;
            match plan.target {
                TargetDto::Stdout => {
                    let out = stdout.ok_or_else(|| {
                        ApiError::Internal("a stdout report without a writer".into())
                    })?;
                    let mut cancel = hikasa_report::CancelWriter::new(out, &job.cancel);
                    let w = render_report(input, &req, &meta, &style, None, &mut cancel)
                        .map_err(|e| render_error(None, e))?;
                    cancel.flush().map_err(ApiError::output)?;
                    outcome.bytes = cancel.written();
                    outcome.warnings.extend(w);
                }
                TargetDto::Clipboard => {
                    let (payload, w) = self.clipboard_payload(input, &req, &meta, &style, job)?;
                    outcome.bytes = payload
                        .html
                        .as_ref()
                        .map_or(payload.text.len(), String::len)
                        as u64;
                    outcome.warnings.extend(w);
                    outcome.clipboard = Some(payload);
                }
                TargetDto::File | TargetDto::Printer | TargetDto::Browser => {
                    let main = plan.main.clone().ok_or_else(|| {
                        ApiError::Internal("a file report without a main file".into())
                    })?;
                    if let Some(folder) = &plan.print_folder {
                        self.prune_print_folders(vfs);
                        vfs.create_dir(folder, true).map_err(ApiError::from_vfs)?;
                    }
                    let result = match &plan.links {
                        None => {
                            let durability = if plan.replaces {
                                Durability::FileOnly
                            } else {
                                Durability::None
                            };
                            let mut warnings = Vec::new();
                            self.stream_into(vfs, &main, job, Some(durability), &mut |out| {
                                warnings = render_report(input, &req, &meta, &style, None, out)
                                    .map_err(|e| render_error(Some(&main), e))?;
                                Ok(())
                            })
                            .map(|bytes| {
                                outcome.bytes = bytes;
                                outcome.warnings.extend(warnings);
                            })
                        }
                        Some(links) => self.write_linked(
                            vfs,
                            &source,
                            input,
                            &req,
                            &meta,
                            &style,
                            &plan,
                            links,
                            &main,
                            job,
                            &mut outcome,
                        ),
                    };
                    if let Err(e) = result {
                        if let Some(folder) = &plan.print_folder {
                            // Nothing of a failed printer report stays below the cache.
                            self.remove_tree(vfs, folder);
                        }
                        return Err(e);
                    }
                    if matches!(plan.target, TargetDto::Printer | TargetDto::Browser) {
                        let path = local_path(&main).ok_or_else(|| {
                            ApiError::Internal("a print file that is not local".into())
                        })?;
                        self.launcher().open_associated(&path).map_err(|_| {
                            ApiError::Unsupported(format!(
                                "no application could open {}",
                                path.display()
                            ))
                        })?;
                        outcome.opened = Some(path);
                    }
                }
            }
            outcome.cancelled = job.cancel.is_cancelled();
            Ok(outcome)
        })
    }

    /// The clipboard payload (§1.7.10): the report's bytes as `text`, or an HTML report as `html` with the same
    /// request rendered as plain text; both together at most [`CLIPBOARD_REPORT_MAX`], else `too_large` at once.
    fn clipboard_payload(
        &self,
        input: ReportInput<'_>,
        req: &ReportRequest,
        meta: &ReportMeta,
        style: &HtmlStyle,
        job: &JobControl,
    ) -> Result<(ClipboardPayload, Vec<Warning>), ApiError> {
        let render =
            |req: &ReportRequest, limit: u64| -> Result<(String, Vec<Warning>), ApiError> {
                let mut cap = CapWriter {
                    buf: Vec::new(),
                    limit,
                    over: false,
                };
                let rendered = {
                    let mut cancel = hikasa_report::CancelWriter::new(&mut cap, &job.cancel);
                    render_report(input, req, meta, style, None, &mut cancel)
                };
                match rendered {
                    Ok(w) => Ok((String::from_utf8_lossy(&cap.buf).into_owned(), w)),
                    Err(_) if cap.over => Err(ApiError::Report(ReportError::TooLarge {
                        limit: CLIPBOARD_REPORT_MAX,
                    })),
                    Err(e) => Err(render_error(None, e)),
                }
            };
        let (main, warnings) = render(req, CLIPBOARD_REPORT_MAX)?;
        if !matches!(req.format(), OutputFormat::Html { .. }) {
            return Ok((
                ClipboardPayload {
                    text: main,
                    html: None,
                },
                warnings,
            ));
        }
        let mut plain = req.clone();
        plain.css = None;
        plain.print = None;
        match &mut plain.body {
            ReportBody::Text(opts) => opts.format = OutputFormat::Text { ansi: false },
            ReportBody::Folder { options, .. } => {
                options.format = OutputFormat::Text { ansi: false };
            }
        }
        let (text, _) = render(&plain, CLIPBOARD_REPORT_MAX - main.len() as u64)?;
        Ok((
            ClipboardPayload {
                text,
                html: Some(main),
            },
            warnings,
        ))
    }

    /// Removes the printer and browser folders below `<cache>/print/` older than seven days by the clock (§1.7.11),
    /// best effort: a failure is a debug log line.
    fn prune_print_folders(&self, vfs: &dyn Vfs) {
        let Ok(root) = self.print_root() else {
            return;
        };
        let options = ListOptions {
            follow: FollowLinks::No,
            prefilter: None,
        };
        let Ok(listing) = vfs.list(&root, &options, &CancelToken::new()) else {
            return;
        };
        let now = self.clock().now();
        let Ok(limit) = now.checked_sub(jiff::Span::new().days(PRINT_KEEP_DAYS)) else {
            return;
        };
        let limit = limit.datetime();
        for entry in listing.entries {
            if entry.kind != EntryKind::Dir {
                continue;
            }
            let Some(at) = print_folder_time(entry.name.as_str()) else {
                continue;
            };
            if at >= limit {
                continue;
            }
            if let Ok(spec) = self.child_of(vfs, &root, &entry.name.to_os_string()) {
                self.remove_tree(vfs, &spec);
            }
        }
    }

    /// Removes a folder and what it holds, best effort (a failure is a debug log line).
    fn remove_tree(&self, vfs: &dyn Vfs, dir: &Spec) {
        let options = ListOptions {
            follow: FollowLinks::No,
            prefilter: None,
        };
        let router = vfs;
        if let Ok(listing) = router.list(dir, &options, &CancelToken::new()) {
            for entry in listing.entries {
                let Ok(child) = self.child_of(vfs, dir, &entry.name.to_os_string()) else {
                    continue;
                };
                if entry.kind == EntryKind::Dir {
                    self.remove_tree(vfs, &child);
                } else if let Err(e) = router.remove(&child) {
                    tracing::debug!(target: "hikasa::report", "cannot remove {}: {e}", child.canonical());
                }
            }
        }
        if let Err(e) = router.remove(dir) {
            tracing::debug!(target: "hikasa::report", "cannot remove {}: {e}", dir.canonical());
        }
    }

    /// Removes the files `names` of `dir`, then `dir` (best effort; `false` when something stayed).
    fn remove_listed(&self, vfs: &dyn Vfs, dir: &Spec, names: &[String]) -> bool {
        let router = vfs;
        let mut ok = true;
        for name in names {
            match self.child_of(vfs, dir, OsStr::new(name)) {
                Ok(child) => ok &= router.remove(&child).is_ok(),
                Err(_) => ok = false,
            }
        }
        ok && router.remove(dir).is_ok()
    }

    /// A free hidden sibling `.<folder>.<what>[-n]` of the files folder.
    fn hidden_sibling(&self, vfs: &dyn Vfs, folder: &Spec, what: &str) -> Result<Spec, ApiError> {
        let dir = folder
            .parent()
            .ok_or_else(|| ApiError::InvalidArgument("the files folder needs a parent".into()))?;
        let name = final_name(folder).unwrap_or_default();
        let name = name.to_string_lossy();
        for n in 1..=999u32 {
            let hidden = if n == 1 {
                format!(".{name}.{what}")
            } else {
                format!(".{name}.{what}-{n}")
            };
            let spec = self.child_of(vfs, &dir, OsStr::new(&hidden))?;
            match vfs.stat(&spec, FollowLinks::No) {
                Err(e) if e.kind() == hikasa_core::ErrorKind::NotFound => return Ok(spec),
                Err(e) => return Err(ApiError::from_vfs(e)),
                Ok(_) => {}
            }
        }
        Err(ApiError::AlreadyExists(format!(
            "no free .{name}.{what} beside {}",
            folder.canonical()
        )))
    }

    /// The pair reports and the main report of a linked plan (§1.7.9 execution steps 1–7).
    #[allow(clippy::too_many_arguments)]
    fn write_linked(
        &self,
        vfs: &dyn Vfs,
        source: &ReportSource<'_>,
        input: ReportInput<'_>,
        req: &ReportRequest,
        meta: &ReportMeta,
        style: &HtmlStyle,
        plan: &ReportPlan,
        links: &LinksPlan,
        main: &Spec,
        job: &JobControl,
        outcome: &mut ReportOutcome,
    ) -> Result<(), ApiError> {
        let router = vfs;
        let cancelled = || job.cancel.check().map_err(|_| ApiError::Cancelled);
        let ReportSource::Folder {
            request, prepared, ..
        } = source
        else {
            return Err(ApiError::Internal(
                "a linked report of a text comparison".into(),
            ));
        };
        let ReportBody::Folder {
            links: Some(file_links),
            ..
        } = &req.body
        else {
            return Err(ApiError::Internal("a linked plan without links".into()));
        };
        job.progress.report(hikasa_core::Progress {
            phase: Phase::Writing,
            done: 0,
            total: Some(links.pairs.len() as u64),
            unit: hikasa_core::Unit::Items,
            current: None,
        });
        // Step 1: the staging folder.
        cancelled()?;
        let staging = self.hidden_sibling(vfs, &links.folder, "hikasa-tmp")?;
        router
            .create_dir(&staging, false)
            .map_err(ApiError::from_vfs)?;
        // The staging folder is this report's own (a free hidden name made here): a failure removes it whole.
        let mut staged: Vec<String> = Vec::new();
        let fail = |_staged: &[String], e: ApiError| -> ApiError {
            self.remove_tree(vfs, &staging);
            e
        };
        // Step 2: the pair reports, then the marker.
        let main_name = final_name(main).unwrap_or_default();
        let back = back_link(&main_name);
        let folder_name = final_name(&links.folder).unwrap_or_default();
        let budget = ByteBudget::new(self.options().content_budget_bytes());
        let pair_req = ReportRequest {
            kind: ReportKind::Text,
            body: ReportBody::Text(file_links.text),
            css: req.css.clone(),
            print: req.print,
            print_mono: req.print_mono,
            title: None,
            scope: ReportScope::All,
            unused: Vec::new(),
        };
        let results: Vec<Result<PairResult, ApiError>> = self.pool().install(|| {
            links
                .pairs
                .par_iter()
                .map(|pair| {
                    self.write_pair(
                        vfs, request, prepared, pair, &pair_req, &staging, &back, meta, style,
                        &budget, job,
                    )
                })
                .collect()
        });
        let mut link_map = BTreeMap::new();
        let mut marker_files = Vec::new();
        for (pair, result) in links.pairs.iter().zip(results) {
            let result = match result {
                Ok(result) => result,
                // A cancel between pairs: nothing stays.
                Err(e) => return Err(fail(&staged, e)),
            };
            let (status, link) = match &result {
                PairResult::Written { size, modified } => {
                    staged.push(pair.file.clone());
                    marker_files.push(MarkerFile {
                        name: pair.file.clone(),
                        size: *size,
                        modified: *modified,
                    });
                    (
                        PairOutcome {
                            path: pair.path.clone(),
                            file: Some(pair.file.clone()),
                            status: PairStatus::Written,
                            reason: None,
                            error: None,
                        },
                        PairLink::Href(href(&[
                            os_bytes(&folder_name),
                            pair.file.clone().into_bytes(),
                        ])),
                    )
                }
                PairResult::Skipped(reason) => (
                    PairOutcome {
                        path: pair.path.clone(),
                        file: None,
                        status: PairStatus::Skipped,
                        reason: Some(reason.clone()),
                        error: None,
                    },
                    PairLink::Skipped(reason.clone()),
                ),
                PairResult::Failed(error) => {
                    outcome.warnings.push(Warning {
                        code: WarningCode::FileReportFailed,
                        message: format!("no file report for {}: {}", pair.path, error.message),
                        spec: None,
                    });
                    (
                        PairOutcome {
                            path: pair.path.clone(),
                            file: None,
                            status: PairStatus::Failed,
                            reason: None,
                            error: Some(error.clone()),
                        },
                        PairLink::Failed(error.message.clone()),
                    )
                }
            };
            outcome.pairs.push(status);
            link_map.insert(pair.node, link);
        }
        cancelled().map_err(|e| fail(&staged, e))?;
        let marker = Marker {
            schema_version: 1,
            report: main_name.to_str().map(str::to_owned),
            report_raw: main_name
                .to_str()
                .is_none()
                .then(|| hex(&os_bytes(&main_name))),
            generator: self.generator(),
            file: marker_files,
        };
        let marker_text = toml::to_string(&marker)
            .map_err(|e| fail(&staged, ApiError::Internal(format!("marker: {e}"))))?;
        let marker_spec = self
            .child_of(vfs, &staging, OsStr::new(MARKER))
            .map_err(|e| fail(&staged, e))?;
        let new_file = WriteOptions {
            create_parents: false,
            overwrite: false,
            replace_link: false,
            durability: Durability::None,
        };
        let written = router
            .open_write(&marker_spec, &new_file)
            .and_then(|mut w| {
                w.write_all(marker_text.as_bytes()).map_err(|e| {
                    VfsError::from_io(marker_spec.canonical(), hikasa_vfs::VfsOp::Write, e)
                })?;
                w.commit()
            });
        written.map_err(|e| fail(&staged, ApiError::from_vfs(e)))?;
        staged.push(MARKER.to_owned());
        // Step 3: the main report into its atomic writer (not committed yet).
        cancelled().map_err(|e| fail(&staged, e))?;
        let main_options = WriteOptions {
            create_parents: false,
            overwrite: true,
            replace_link: false,
            durability: if plan.replaces {
                Durability::FileOnly
            } else {
                Durability::None
            },
        };
        let mut writer = router
            .open_write(main, &main_options)
            .map_err(|e| fail(&staged, ApiError::from_vfs(e)))?;
        let plan_links = PlanLinks { links: link_map };
        let rendered = {
            let mut buffered = std::io::BufWriter::with_capacity(REPORT_BUFFER, &mut writer);
            let result = {
                let mut cancel = hikasa_report::CancelWriter::new(&mut buffered, &job.cancel);
                render_report(input, req, meta, style, Some(&plan_links), &mut cancel)
                    .map(|w| (w, cancel.written()))
                    .map_err(|e| render_error(Some(main), e))
            };
            result.and_then(|r| {
                buffered
                    .flush()
                    .map(|()| r)
                    .map_err(|e| write_error(main, e))
            })
        };
        let (warnings, bytes) = rendered.map_err(|e| fail(&staged, e))?;
        outcome.bytes = bytes;
        outcome.warnings.extend(warnings);
        // Step 4: the old folder, unchanged since the plan, out of the way.
        cancelled().map_err(|e| fail(&staged, e))?;
        let old = match links.state {
            FilesFolder::New => None,
            FilesFolder::ReplaceOurs => {
                if self.folder_stamps(vfs, &links.folder).as_deref() != Some(&links.stamps[..]) {
                    return Err(fail(
                        &staged,
                        ApiError::InvalidData(format!(
                            "{} changed since the plan was made",
                            links.folder.canonical()
                        )),
                    ));
                }
                let old = self
                    .hidden_sibling(vfs, &links.folder, "hikasa-old")
                    .map_err(|e| fail(&staged, e))?;
                router
                    .rename(&links.folder, &old, false)
                    .map_err(|e| fail(&staged, ApiError::from_vfs(e)))?;
                Some(old)
            }
        };
        let put_back = |old: &Option<Spec>| {
            if let Some(old) = old
                && let Err(e) = router.rename(old, &links.folder, false)
            {
                tracing::debug!(target: "hikasa::report", "cannot rename {} back: {e}", old.canonical());
            }
        };
        // Step 5: the staging folder becomes the files folder. From here the report runs to its end.
        if let Err(e) = router.rename(&staging, &links.folder, false) {
            let e = fail(&staged, ApiError::from_vfs(e));
            put_back(&old);
            return Err(e);
        }
        // Step 6: the main file.
        if let Err(e) = writer.commit() {
            if router.rename(&links.folder, &staging, false).is_ok() {
                self.remove_tree(vfs, &staging);
            }
            put_back(&old);
            return Err(ApiError::from_vfs(e));
        }
        // Step 7: the old folder goes.
        if let Some(old) = old {
            let names: Vec<String> = links.stamps.iter().map(|(n, _)| n.clone()).collect();
            if !self.remove_listed(vfs, &old, &names) {
                outcome.warnings.push(Warning {
                    code: WarningCode::ReportOldFolderKept,
                    message: format!(
                        "the earlier pair reports in {} could not be removed",
                        old.canonical()
                    ),
                    spec: None,
                });
            }
        }
        Ok(())
    }

    /// One pair report into `staging` (§1.7.8): compared through the content budget, rendered, written, dropped.
    #[allow(clippy::too_many_arguments)]
    fn write_pair(
        &self,
        vfs: &dyn Vfs,
        request: &super::requests::FolderCompareRequest,
        prepared: &super::responses::PreparedFolderCompare,
        pair: &PairPlan,
        pair_req: &ReportRequest,
        staging: &Spec,
        back: &FolderLink,
        folder_meta: &ReportMeta,
        style: &HtmlStyle,
        budget: &ByteBudget,
        job: &JobControl,
    ) -> Result<PairResult, ApiError> {
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        if let Some(reason) = &pair.skip {
            return Ok(PairResult::Skipped(reason.clone()));
        }
        let Some(text_request) = self.pair_text_request(request, prepared, pair.node) else {
            return Ok(PairResult::Skipped("not_a_file_pair".into()));
        };
        let sizes = {
            let read = prepared.model().read();
            read.node(pair.node).map_or(0, |view| {
                view.left.as_ref().map_or(0, |e| e.size) + view.right.as_ref().map_or(0, |e| e.size)
            })
        };
        let _held = budget
            .acquire(sizes.min(budget.capacity()), &job.cancel)
            .map_err(|_| ApiError::Cancelled)?;
        let sub = JobControl {
            progress: std::sync::Arc::new(hikasa_core::NullProgress),
            ..job.clone()
        };
        let resp = match self.text_compare(&text_request, &sub, None) {
            Ok(resp) => resp,
            Err(ApiError::Cancelled) => return Err(ApiError::Cancelled),
            Err(e) => return Ok(PairResult::Failed(e.report())),
        };
        if resp
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::BinaryContent)
        {
            return Ok(PairResult::Skipped("binary".into()));
        }
        let mut req = pair_req.clone();
        if let ReportBody::Text(opts) = &mut req.body {
            opts.ignore_unimportant = resp.ignore_unimportant;
        }
        let ReportBody::Text(opts) = &req.body else {
            return Err(ApiError::Internal(
                "a pair request that is not a text one".into(),
            ));
        };
        let mut meta = ReportMeta::for_text(
            &resp.comparison,
            opts,
            folder_meta.generated_at.clone(),
            &self.generator(),
        );
        meta.folder_link = Some(back.clone());
        let spec = self.child_of(vfs, staging, OsStr::new(&pair.file))?;
        let options = WriteOptions {
            create_parents: false,
            overwrite: false,
            replace_link: false,
            durability: Durability::None,
        };
        let router = vfs;
        let written = router
            .open_write(&spec, &options)
            .map_err(ApiError::from_vfs)
            .and_then(|mut writer| {
                {
                    let mut buffered =
                        std::io::BufWriter::with_capacity(REPORT_BUFFER, &mut writer);
                    let mut cancel = hikasa_report::CancelWriter::new(&mut buffered, &job.cancel);
                    render_report(
                        ReportInput::Text(&resp.comparison),
                        &req,
                        &meta,
                        style,
                        None,
                        &mut cancel,
                    )
                    .map_err(|e| render_error(Some(&spec), e))?;
                    buffered.flush().map_err(|e| write_error(&spec, e))?;
                }
                writer.commit().map_err(ApiError::from_vfs)
            });
        match written {
            Ok(()) => {}
            Err(ApiError::Cancelled) => return Err(ApiError::Cancelled),
            Err(e) => {
                let _ = router.remove(&spec);
                return Ok(PairResult::Failed(e.report()));
            }
        }
        let entry = router
            .stat(&spec, FollowLinks::No)
            .map_err(ApiError::from_vfs)?;
        Ok(PairResult::Written {
            size: entry.size,
            modified: entry.mtime.map(nanos),
        })
    }
}

/// What one pair's report became.
enum PairResult {
    Written { size: u64, modified: Option<i64> },
    Skipped(String),
    Failed(ErrorReport),
}

/// The back link of a pair report (§1.7.8): `../<main final name>` from the OS bytes on Unix, from the name on Windows
/// when it is valid Unicode; otherwise the name without a link.
fn back_link(main_name: &OsStr) -> FolderLink {
    let name = main_name.to_string_lossy().into_owned();
    let bytes = if cfg!(windows) {
        main_name.to_str().map(|s| s.as_bytes().to_vec())
    } else {
        Some(os_bytes(main_name))
    };
    FolderLink {
        name,
        href: bytes.map(|b| href(&[b"..".to_vec(), b])),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A7.9: the `safe_component` table.
    #[test]
    fn a7_9_safe_component_table() {
        let table: [(&str, &str); 26] = [
            ("main.c", "main.c"),
            ("a\u{1}b\u{1b}c", "a_b_c"),
            ("a/b\\c", "a_b_c"),
            ("x:y*z?", "x_y_z_"),
            ("\"q\"<l>|p", "_q__l__p"),
            ("r#1%2", "r_1_2"),
            (".hidden", "_hidden"),
            ("-rf", "_rf"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("NUL.x", "_NUL.x"),
            ("COM1", "_COM1"),
            ("lpt9.tar.gz", "_lpt9.tar.gz"),
            ("name.", "name_"),
            ("bad\u{fffd}.txt", "bad_.txt"),
            ("", "file"),
            ("é日本", "é日本"),
            ("a b", "a_b"),
            ("tab\there", "tab_here"),
            ("\u{202e}txt.exe", "_txt.exe"),
            ("..", "__"),
            ("...", "_._"),
            ("~home", "_home"),
            ("a+b=c", "a_b_c"),
            ("CONSOLE", "CONSOLE"),
            ("CON .txt", "CON_.txt"),
        ];
        for (name, want) in table {
            let got = safe_component(name, 64);
            assert_eq!(got, want, "{name:?}");
            assert_eq!(windows_name_problem(&got), None, "{name:?} → {got:?}");
        }
        // 300 bytes of `é` (2 bytes each): cut at a char boundary to 64 bytes.
        let long = "é".repeat(150);
        let got = safe_component(&long, 64);
        assert_eq!(got.len(), 64);
        assert_eq!(got, "é".repeat(32));
        let got = safe_component(&long, 63);
        assert_eq!(got.len(), 62, "never half a char");
        // A cut that makes a device name or a trailing dot is fixed after the cut.
        assert_eq!(safe_component("CONxyz", 3), "_CO");
        assert_eq!(safe_component("ab.cd", 3), "ab_");
        // The device prefix moves a dot to the end of the cut: `_CON.` is refused, so it becomes `_`.
        for (name, max, want) in [
            ("CON.x", 5, "_CON_"),
            ("nul.tar", 5, "_nul_"),
            ("COM1.c", 6, "_COM1_"),
        ] {
            let got = safe_component(name, max);
            assert_eq!(got, want, "{name:?} at {max}");
            assert_eq!(windows_name_problem(&got), None, "{name:?} → {got:?}");
        }
    }

    #[test]
    fn a7_9_names_of_the_files() {
        assert_eq!(main_stem(OsStr::new("report.html")), "report");
        assert_eq!(main_stem(OsStr::new(".html")), "report");
        assert_eq!(main_stem(OsStr::new("a.b.html")), "a.b");
        assert_eq!(main_stem(OsStr::new("noext")), "noext");
        assert_eq!(
            files_folder_name(OsStr::new("report.html"), 1),
            "report_files"
        );
        assert_eq!(
            files_folder_name(OsStr::new("my report.html"), 2),
            "my_report_files-2"
        );
        assert_eq!(files_folder_name(OsStr::new("CON.html"), 1), "_CON_files");
        // Zero padding to the digits of the number of linked rows.
        assert_eq!(pair_file_name(1, 9, OsStr::new("a.c")), "1-a.c.html");
        assert_eq!(pair_file_name(9, 9, OsStr::new("a.c")), "9-a.c.html");
        assert_eq!(pair_file_name(1, 10, OsStr::new("a.c")), "01-a.c.html");
        assert_eq!(pair_file_name(10, 10, OsStr::new("a.c")), "10-a.c.html");
        assert_eq!(pair_file_name(7, 1_000, OsStr::new("x")), "0007-x.html");
        assert_eq!(pair_file_name(1_000, 1_000, OsStr::new("x")), "1000-x.html");
        // Two pairs whose names differ in case only get names whose case folds differ.
        let (a, b) = (
            pair_file_name(1, 2, OsStr::new("A.txt")),
            pair_file_name(2, 2, OsStr::new("a.txt")),
        );
        assert_ne!(a.to_lowercase(), b.to_lowercase());
        assert_eq!((a.as_str(), b.as_str()), ("1-A.txt.html", "2-a.txt.html"));
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]

        /// P-LINKS, the names: over random names, the safe name passes `windows_name_problem`, fits `max` bytes, is never
        /// empty, and holds no byte an href escapes but those of non-ASCII letters and digits. Half the cases cut the name
        /// at or just after one of its dots (counting the device prefix), where a cut leaves a trailing dot.
        #[test]
        fn p_links_safe_names(
            name in "\\PC{0,40}|[.\\-]{0,3}(CON|con|NUL|com1|LPT2)(\\.[a-z]{0,3})?\\.?|[a-z.]{0,6}\\.[a-z.]{0,6}",
            max in 1usize..80,
            near_dot in proptest::option::of((proptest::prelude::any::<proptest::sample::Index>(), 0usize..3)),
        ) {
            let dots: Vec<usize> = name.match_indices('.').map(|(at, _)| at).collect();
            let max = match near_dot {
                Some((which, near)) if !dots.is_empty() => dots[which.index(dots.len())] + near + 1,
                _ => max,
            };
            let safe = safe_component(&name, max);
            proptest::prop_assert!(windows_name_problem(&safe).is_none(), "{name:?} → {safe:?}");
            proptest::prop_assert!(!safe.is_empty() && safe.len() <= max.max(4), "{name:?} → {safe:?}");
            proptest::prop_assert!(!safe.starts_with(['.', '-']));
            for c in safe.chars() {
                proptest::prop_assert!(
                    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') || (!c.is_ascii() && c.is_alphanumeric()),
                    "{name:?} → {safe:?}"
                );
            }
        }
    }
}
