//! Reports and output files (§3.2, §3.4; RPT-004, RPT-011, D-24): [`Service::render_text`],
//! [`Service::render_folder`], [`Service::write_output`] and their `_with` variants with [`ReportSettings`]; the M2
//! W7 report requests (§1.7.14): [`ReportTarget`], [`ReportFlags`], [`ReportSource`], [`Service::report_request`],
//! [`Service::pair_text_request`], [`Service::report_time`], the remembered choices and
//! [`Service::write_output_with`]. Plans and their execution are in [`super::report_write`].

use std::ffi::{OsStr, OsString};
use std::io::Write;

use hikasa_core::{Side, Spec, Warning, WarningCode};
use hikasa_folder::NodeKind;
use hikasa_folder::{NodeId, select::Selection};
use hikasa_report::{
    CancelWriter, FolderLayout, FolderReportOptions, HtmlStyle, LineScope, ReportBody,
    ReportDefaults, ReportError, ReportKind, ReportMeta, ReportRequest, ReportScope, ReportWords,
    TargetKind, TextReportOptions, render_folder_report_with, render_text_report_with,
};
use hikasa_vfs::{Durability, Vfs, VfsError, VfsOp, WriteOptions};
use serde::{Deserialize, Serialize};

use super::folder_ops::SelectRequest;
use super::requests::{FolderCompareRequest, TextCompareRequest};
use super::responses::{FolderCompareResponse, PreparedFolderCompare, TextCompareResponse};
use super::vocab::{parse_line_range, resolve_folder_display, resolve_text_display};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};
use crate::settings::LoadMode;
use crate::settings::state::State;

/// The buffer between a renderer and its target (§1.7.9): 64 KiB.
pub(crate) const REPORT_BUFFER: usize = 64 * 1024;

/// Where a report goes (§1.7.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportTarget {
    /// Standard output (`-`, the default).
    Stdout,
    /// A file spec, as typed (`--output-to <FILE>`, `--output <FILE>`).
    File(OsString),
    /// The clipboard (`clipboard`, `clipboard://`).
    Clipboard,
    /// A new file below `<cache>/print/`, opened by the associated application with the print block (`printer`).
    Printer,
    /// As `printer`, without the print block (`browser`).
    Browser,
}

impl ReportTarget {
    /// `-` → Stdout; `clipboard` or `clipboard://` → Clipboard; `printer`, `browser`; else a file spec (`./printer` is a
    /// file). The keywords are exact (lower case): `Printer` is a file too.
    pub fn parse(arg: &OsStr) -> ReportTarget {
        match arg.to_str() {
            Some("-") => ReportTarget::Stdout,
            Some("clipboard" | "clipboard://") => ReportTarget::Clipboard,
            Some("printer") => ReportTarget::Printer,
            Some("browser") => ReportTarget::Browser,
            _ => ReportTarget::File(arg.to_owned()),
        }
    }

    /// The target as `hikasa-report`'s matrices know it; `ansi` for a terminal's stdout.
    pub fn kind(&self, ansi: bool) -> TargetKind {
        match self {
            ReportTarget::Stdout => TargetKind::Stdout { ansi },
            ReportTarget::File(_) => TargetKind::File,
            ReportTarget::Clipboard => TargetKind::Clipboard,
            ReportTarget::Printer => TargetKind::Printer,
            ReportTarget::Browser => TargetKind::Browser,
        }
    }

    /// The serialisable kind of the target.
    pub fn dto(&self) -> TargetDto {
        match self {
            ReportTarget::Stdout => TargetDto::Stdout,
            ReportTarget::File(_) => TargetDto::File,
            ReportTarget::Clipboard => TargetDto::Clipboard,
            ReportTarget::Printer => TargetDto::Printer,
            ReportTarget::Browser => TargetDto::Browser,
        }
    }
}

/// The kind of a report target, as `hikasa.report.v1` writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetDto {
    /// Standard output.
    Stdout,
    /// A file.
    File,
    /// The clipboard.
    Clipboard,
    /// The printer (the associated application of a file below `<cache>/print/`).
    Printer,
    /// The browser (as the printer, without the print block).
    Browser,
}

impl TargetDto {
    /// `stdout`, `file`, `clipboard`, `printer`, `browser`.
    pub const fn as_str(self) -> &'static str {
        match self {
            TargetDto::Stdout => "stdout",
            TargetDto::File => "file",
            TargetDto::Clipboard => "clipboard",
            TargetDto::Printer => "printer",
            TargetDto::Browser => "browser",
        }
    }
}

/// The CLI's report flags, raw (§1.7.14); the Service turns them into words and defaults (T11).
#[derive(Debug, Clone, Default)]
pub struct ReportFlags {
    /// `--layout`/`--report`, `--options`, `--output-options`, `--title`.
    pub words: ReportWords,
    /// `--html` (`Some(false)`) or `--html-mono` (`Some(true)`): the word `html-color` or `html-mono`.
    pub html: Option<bool>,
    /// `--wrap <MODE>`: the word `wrap-<MODE>` (text).
    pub wrap: Option<String>,
    /// `--patch-style <S>`: the word `patch-<S>` (text).
    pub patch_style: Option<String>,
    /// `--columns <LIST>`: a word `column-<item>` per item (folders).
    pub columns: Option<String>,
    /// `--line-numbers` (`Some(true)`), `--no-line-numbers` (`Some(false)`) (text).
    pub line_numbers: Option<bool>,
    /// `--display <WORD>`: the display of an absent `display-*` word (M1's words and defaults).
    pub display: Option<String>,
    /// `--folders <MODE>` (folders): the folder mode of the default display.
    pub folders: Option<String>,
    /// `--suppress-filters` (folders).
    pub suppress_filters: bool,
    /// `--context <N>` (text): the lines of `display-context` and of the patch styles; alone, `display-context`.
    pub context: Option<u32>,
    /// `--width <N>` (text).
    pub width: Option<u16>,
    /// `--lines [left:|right:]<A>-<B>` (text): the scope.
    pub lines: Option<String>,
    /// `--select <MASK>…`, `--path <REL>…` (folders): the scope, resolved by [`Service::report_plan`].
    pub select: Option<SelectRequest>,
    /// The terminal takes ANSI colours (stdout only; W7b: the adapter knows it, `report_request` applies it).
    pub ansi: bool,
}

/// The comparison a report shows (§1.7.14). A folder report keeps the request and the prepared comparison, which
/// the pair requests of a linked report need.
#[derive(Clone, Copy)]
pub enum ReportSource<'a> {
    /// A text comparison.
    Text(&'a TextCompareResponse),
    /// A folder comparison.
    Folder {
        /// Its request.
        request: &'a FolderCompareRequest,
        /// Its prepared comparison (`folder_prepare_in`).
        prepared: &'a PreparedFolderCompare,
        /// Its result (`folder_run`).
        response: &'a FolderCompareResponse,
    },
}

impl ReportSource<'_> {
    /// The kind of the source.
    pub const fn kind(&self) -> ReportKind {
        match self {
            ReportSource::Text(_) => ReportKind::Text,
            ReportSource::Folder { .. } => ReportKind::Folder,
        }
    }

    /// The verdict, as in `hikasa.text_compare.v1` and `hikasa.folder_compare.v1`.
    pub fn verdict(&self) -> ReportVerdict {
        match self {
            ReportSource::Text(resp) => {
                ReportVerdict::Text(resp.comparison.verdict(resp.ignore_unimportant))
            }
            ReportSource::Folder { response, .. } => {
                ReportVerdict::Folder(response.verdict.as_str().to_owned())
            }
        }
    }
}

/// The verdict of a report's comparison, serialised as `hikasa.text_compare.v1` and `hikasa.folder_compare.v1` write
/// it; `None` (`null`) for a dry run, which keeps no comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ReportVerdict {
    /// A text comparison's `{result, …}`.
    Text(hikasa_core::ContentResult),
    /// A folder comparison's word.
    Folder(String),
    /// Not known.
    #[default]
    None,
}

/// The remembered choices of the report dialog of one kind (§1.7.13, `[dialogs."report-text"]`,
/// `[dialogs."report-folder"]`). The title is not remembered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportChoices {
    /// The layout word.
    pub layout: Option<String>,
    /// The option words.
    pub options: Vec<String>,
    /// The output words.
    pub output_options: Vec<String>,
    /// `file`, `clipboard`, `printer` or `browser`.
    pub output_to: Option<String>,
    /// The last file target, as typed.
    pub file: Option<String>,
}

/// The `[dialogs]` table of a kind's choices.
fn choices_table(kind: ReportKind) -> &'static str {
    match kind {
        ReportKind::Text => "report-text",
        ReportKind::Folder => "report-folder",
    }
}

/// A flag of the other kind (§1.7.15: the five commands parse alike; `report_request` refuses).
fn other_kind(flag: &str, kind: ReportKind) -> ApiError {
    let other = match kind {
        ReportKind::Text => "folder",
        ReportKind::Folder => "text",
    };
    ApiError::InvalidArgument(format!("{flag} applies to {other} reports"))
}

/// Report inputs that the option structs of hikasa-report do not carry (additive, wave 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSettings {
    /// `--title <text>`: the report title (`None` = "Text Compare" / "Folder Compare"; RPT-004).
    pub title: Option<String>,
    /// HTML styling (`html-custom` stylesheet).
    pub style: HtmlStyle,
    /// Folder `List` layout: write the closing `# <n> same, …` summary line (§4.3.3). Default `true`, and no
    /// CLI flag changes it: `--quiet` keeps the line (§4.3.3 amendment, CLI invariant 3: `--quiet` never changes
    /// stdout). `false` is an API-only option for embedders that show the counts elsewhere.
    pub list_summary: bool,
}

impl Default for ReportSettings {
    /// No title, the built-in stylesheet, the list summary line written.
    fn default() -> ReportSettings {
        ReportSettings {
            title: None,
            style: HtmlStyle::default(),
            list_summary: true,
        }
    }
}

impl Service {
    /// Renders a text report with `ReportMeta::for_text(…, Some(clock.now()), "hikasa <version>")`.
    pub fn render_text(
        &self,
        resp: &TextCompareResponse,
        opts: &TextReportOptions,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        self.render_text_with(resp, opts, &ReportSettings::default(), out)
            .map(|_| ())
    }

    /// [`Service::render_text`] with a title and HTML style; returns the warnings of the rendering itself
    /// (`patch_lossy_eol`), which the CLI adds to its JSON/stderr warnings. Additive.
    ///
    /// `opts.ignore_unimportant` is replaced by the response's flag, so the report agrees with the exit code
    /// (wave-4 review). The title goes through `ReportMeta::with_title`.
    pub fn render_text_with(
        &self,
        resp: &TextCompareResponse,
        opts: &TextReportOptions,
        settings: &ReportSettings,
        out: &mut dyn Write,
    ) -> Result<Vec<Warning>, ApiError> {
        run_guarded(|| {
            let opts = TextReportOptions {
                ignore_unimportant: resp.ignore_unimportant,
                ..*opts
            };
            let mut meta = ReportMeta::for_text(
                &resp.comparison,
                &opts,
                Some(self.clock().now()),
                &self.generator(),
            );
            if let Some(title) = &settings.title {
                meta = meta.with_title(title.clone());
            }
            Ok(render_text_report_with(
                &resp.comparison,
                &opts,
                &meta,
                &settings.style,
                out,
            )?)
        })
    }

    /// Renders a folder report with `ReportMeta::for_folder(…)`.
    pub fn render_folder(
        &self,
        resp: &FolderCompareResponse,
        opts: &FolderReportOptions,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        self.render_folder_with(resp, opts, &ReportSettings::default(), out)
    }

    /// [`Service::render_folder`] with a title, an HTML style and the list summary switch. Additive.
    ///
    /// `opts.display.ignore_unimportant` is replaced by the response's effective flag (the request's flag or the
    /// criteria word), so categories, markers and the header agree with the verdict and the exit code
    /// (decision 4 of the wave-4 review). The title goes through `ReportMeta::with_title`. hikasa-report has no
    /// switch for the list's summary line; with `list_summary == false` (API only) the service drops the `# …`
    /// line from the `List` output (list rows start with a marker, never with `# `).
    pub fn render_folder_with(
        &self,
        resp: &FolderCompareResponse,
        opts: &FolderReportOptions,
        settings: &ReportSettings,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        run_guarded(|| {
            let mut opts = *opts;
            opts.display.ignore_unimportant = resp.ignore_unimportant;
            let opts = &opts;
            let mut meta = ReportMeta::for_folder(
                &resp.comparison,
                opts,
                Some(self.clock().now()),
                &self.generator(),
            );
            if let Some(title) = &settings.title {
                meta = meta.with_title(title.clone());
            }
            if opts.layout == FolderLayout::List && !settings.list_summary {
                let mut filter = DropSummaryLines::new(out);
                render_folder_report_with(
                    &resp.comparison,
                    opts,
                    &meta,
                    &settings.style,
                    &mut filter,
                )?;
                filter.finish().map_err(|e| ApiError::Report(e.into()))?;
                Ok(())
            } else {
                Ok(render_folder_report_with(
                    &resp.comparison,
                    opts,
                    &meta,
                    &settings.style,
                    out,
                )?)
            }
        })
    }

    /// The request of a report (§1.7.14): the flags become words appended after `flags.words` (`--html`, `--wrap`,
    /// `--patch-style`, `--columns`, `--line-numbers`), the displays and numbers become defaults (`--display`,
    /// `--folders`, `--suppress-filters`, `--context`, `--width`), then [`ReportRequest::parse`],
    /// [`ReportRequest::check_target`] and [`ReportRequest::apply_target`]. Runs before the comparison, so a refused
    /// word, combination or target exits 100 with nothing compared. A flag of the other kind is `invalid_input`.
    pub fn report_request(
        &self,
        kind: ReportKind,
        default_layout: &'static str,
        flags: &ReportFlags,
        target: &ReportTarget,
    ) -> Result<ReportRequest, ApiError> {
        run_guarded(|| {
            match kind {
                ReportKind::Text => {
                    if flags.folders.is_some() {
                        return Err(other_kind("--folders", kind));
                    }
                    if flags.suppress_filters {
                        return Err(other_kind("--suppress-filters", kind));
                    }
                    if flags.columns.is_some() {
                        return Err(other_kind("--columns", kind));
                    }
                    if flags.select.is_some() {
                        return Err(other_kind("--select and --path", kind));
                    }
                }
                ReportKind::Folder => {
                    for (set, flag) in [
                        (flags.patch_style.is_some(), "--patch-style"),
                        (flags.wrap.is_some(), "--wrap"),
                        (
                            flags.line_numbers.is_some(),
                            "--line-numbers and --no-line-numbers",
                        ),
                        (flags.context.is_some(), "--context"),
                        (flags.width.is_some(), "--width"),
                        (flags.lines.is_some(), "--lines"),
                    ] {
                        if set {
                            return Err(other_kind(flag, kind));
                        }
                    }
                }
            }
            let mut words = flags.words.clone();
            if let Some(mono) = flags.html {
                let word = if mono { "html-mono" } else { "html-color" };
                words.output_options.push(word.to_owned());
            }
            if let Some(wrap) = &flags.wrap {
                words.output_options.push(format!("wrap-{}", wrap.trim()));
            }
            if let Some(style) = &flags.patch_style {
                words.options.push(format!("patch-{}", style.trim()));
            }
            if let Some(on) = flags.line_numbers {
                let word = if on {
                    "line-numbers"
                } else {
                    "no-line-numbers"
                };
                words.options.push(word.to_owned());
            }
            if let Some(list) = &flags.columns {
                for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
                    words.options.push(format!("column-{item}"));
                }
            }
            let default_context = self.options().context_lines;
            let mut defaults = ReportDefaults::new(default_layout);
            defaults.context_lines = flags.context.unwrap_or(default_context);
            if let Some(width) = flags.width {
                defaults.width = width;
            }
            match kind {
                ReportKind::Text => {
                    defaults.text_display = resolve_text_display(
                        flags.display.as_deref(),
                        flags.context,
                        default_context,
                        false,
                    )?;
                }
                ReportKind::Folder => {
                    defaults.folder_display = resolve_folder_display(
                        flags.display.as_deref(),
                        flags.folders.as_deref(),
                        flags.suppress_filters,
                        false,
                    )?;
                }
            }
            let mut req = ReportRequest::parse(kind, &words, &defaults)?;
            // M1's flags (`--wrap`, `--patch-style`, `--line-numbers`, `--columns`, `--html`, …) were silent where the
            // layout does not use them, and stay so (W7b review): only the words given as words are reported unused.
            // A part of the words parses whenever all of them do (fewer words, fewer conflicts).
            if let Ok(own) = ReportRequest::parse(kind, &flags.words, &defaults) {
                req.unused.retain(|word| own.unused.contains(word));
            }
            if let Some(lines) = &flags.lines {
                req.scope = ReportScope::TextLines(line_scope(lines)?);
            }
            if flags.select.is_some() {
                // The rows are known after the comparison (`report_plan`); the layout is checked now.
                req.scope = ReportScope::FolderRows(Selection::default());
            }
            let kind_of = target.kind(flags.ansi);
            req.check_target(kind_of)?;
            // Links into an archive: refused here, before anything is compared, as links to stdout or the clipboard.
            if let (ReportTarget::File(raw), ReportBody::Folder { links: Some(_), .. }) =
                (target, &req.body)
                && !matches!(self.write_spec(raw)?, Spec::Local(_))
            {
                return Err(ApiError::Report(ReportError::Refused {
                    message: "file reports need a folder on disk",
                }));
            }
            req.apply_target(kind_of);
            Ok(req)
        })
    }

    /// The one builder of a pair's Text Compare request (§1.7.8, T11): both sides' exact specs
    /// ([`TextCompareRequest::for_specs`]), the folder request's text rules file and patch, the prepared format
    /// overrides (FF-002) and the effective ignore-unimportant. `None` for a row that is not a file pair (a folder, an
    /// orphan, a link shown as a link, an unknown node).
    pub fn pair_text_request(
        &self,
        req: &FolderCompareRequest,
        prepared: &PreparedFolderCompare,
        node: NodeId,
    ) -> Option<TextCompareRequest> {
        let read = prepared.model().read();
        let view = read.node(node)?;
        if view.kind != NodeKind::File || view.left.is_none() || view.right.is_none() {
            return None;
        }
        let (left, right) = (read.spec(node, Side::Left)?, read.spec(node, Side::Right)?);
        let mut request = TextCompareRequest::for_specs(&left, &right);
        request.rules_file.clone_from(&req.text_rules_file);
        request.rules_patch = req.text_patch.clone();
        request.ignore_unimportant = prepared.ignore_unimportant();
        request
            .format_overrides
            .clone_from(prepared.format_overrides());
        Some(request)
    }

    /// The time of a report (D-206): `SOURCE_DATE_EPOCH` (a decimal count of seconds, shown in UTC) when it is set,
    /// else the clock in the user's zone. A value that does not parse, or lies outside jiff's range, is ignored with
    /// one warning `source_date_epoch_invalid` (in [`Service::take_warnings`]). One report job takes it once.
    pub fn report_time(&self) -> Option<jiff::Zoned> {
        if let Some(raw) = &self.env().source_date_epoch {
            match source_date_epoch(raw) {
                Some(at) => return Some(at),
                None => self.note(Warning {
                    code: WarningCode::SourceDateEpochInvalid,
                    message: format!(
                        "SOURCE_DATE_EPOCH {raw:?} is not a count of seconds since 1970 in range; the clock gives the report time"
                    ),
                    spec: None,
                }),
            }
        }
        Some(self.clock().now())
    }

    /// The remembered choices of the report dialog of `kind` (§1.7.13), read leniently: a word
    /// [`ReportRequest::parse`] refuses (alone, or against a word before it) is dropped with one warning
    /// `invalid_config_value` (in [`Service::take_warnings`]); a missing table or file gives the defaults.
    pub fn report_choices(&self, kind: ReportKind) -> ReportChoices {
        let table = choices_table(kind);
        let state = self
            .settings_store()
            .file::<State>(LoadMode::Lenient)
            .map(|file| file.value().clone())
            .unwrap_or_default();
        let Some(dialog) = state.dialogs.get(table).and_then(toml::Value::as_table) else {
            return ReportChoices::default();
        };
        let mut dropped: Vec<String> = Vec::new();
        let text = |key: &str, dropped: &mut Vec<String>| match dialog.get(key) {
            None => None,
            Some(toml::Value::String(s)) => Some(s.clone()),
            Some(other) => {
                dropped.push(format!("{key} = {other}"));
                None
            }
        };
        let list = |key: &str, dropped: &mut Vec<String>| -> Vec<String> {
            match dialog.get(key) {
                None => Vec::new(),
                Some(toml::Value::Array(items)) => items
                    .iter()
                    .filter_map(|item| match item {
                        toml::Value::String(s) => Some(s.clone()),
                        other => {
                            dropped.push(format!("{key}: {other}"));
                            None
                        }
                    })
                    .collect(),
                Some(other) => {
                    dropped.push(format!("{key} = {other}"));
                    Vec::new()
                }
            }
        };
        let defaults = ReportDefaults::new("side-by-side");
        let mut choices = ReportChoices {
            layout: text("layout", &mut dropped),
            output_to: text("output_to", &mut dropped),
            file: text("file", &mut dropped),
            ..ReportChoices::default()
        };
        if let Some(layout) = &choices.layout {
            let words = ReportWords {
                layout: Some(layout.clone()),
                ..ReportWords::default()
            };
            if ReportRequest::parse(kind, &words, &defaults).is_err() {
                dropped.push(format!("layout {layout}"));
                choices.layout = None;
            }
        }
        // Each word is checked against the words kept before it, under side-by-side (the layout that uses the most
        // slots): the remembered words span layouts (D-208), so a word another layout does not use stays.
        let mut kept = ReportWords {
            layout: Some("side-by-side".to_owned()),
            ..ReportWords::default()
        };
        for (key, output) in [("options", false), ("output_options", true)] {
            for word in list(key, &mut dropped) {
                let mut trial = kept.clone();
                if output {
                    trial.output_options.push(word.clone());
                } else {
                    trial.options.push(word.clone());
                }
                if ReportRequest::parse(kind, &trial, &defaults).is_ok() {
                    kept = trial;
                    if output {
                        choices.output_options.push(word);
                    } else {
                        choices.options.push(word);
                    }
                } else {
                    dropped.push(word);
                }
            }
        }
        if let Some(to) = &choices.output_to
            && !matches!(to.as_str(), "file" | "clipboard" | "printer" | "browser")
        {
            dropped.push(format!("output_to {to}"));
            choices.output_to = None;
        }
        if !dropped.is_empty() {
            self.note(Warning {
                code: WarningCode::InvalidConfigValue,
                message: format!(
                    "state.toml [dialogs.\"{table}\"]: dropped {}",
                    dropped.join(", ")
                ),
                spec: None,
            });
        }
        choices
    }

    /// Remembers the choices of the report dialog of `kind` (§1.7.13) through `state_update` (D-108): the table is
    /// written sparse (absent values and empty lists are left out); the other `[dialogs]` tables stay.
    pub fn remember_report_choices(
        &self,
        kind: ReportKind,
        choices: &ReportChoices,
    ) -> Result<(), ApiError> {
        let table = choices_table(kind);
        let mut value = toml::Table::new();
        let mut put_text = |key: &str, text: &Option<String>| {
            if let Some(text) = text {
                value.insert(key.to_owned(), toml::Value::String(text.clone()));
            }
        };
        put_text("layout", &choices.layout);
        put_text("output_to", &choices.output_to);
        put_text("file", &choices.file);
        for (key, list) in [
            ("options", &choices.options),
            ("output_options", &choices.output_options),
        ] {
            if !list.is_empty() {
                value.insert(
                    key.to_owned(),
                    toml::Value::Array(list.iter().cloned().map(toml::Value::String).collect()),
                );
            }
        }
        self.state_update(&mut |state| {
            state
                .dialogs
                .insert(table.to_owned(), toml::Value::Table(value.clone()));
        })
        .map(|_| ())
    }

    /// Streams a report into a spec atomically (D-202): the VFS's atomic writer behind a 64 KiB buffer and a
    /// [`CancelWriter`] on the job's token; nothing holds the report. A render error or a cancel drops the writer, so
    /// the target is untouched (M1 `--output` semantics, also for a link target, a FIFO and a device). Returns the
    /// spec and the bytes written. The new file is written with `Durability::None`, a replaced one `FileOnly` (D-164).
    pub fn write_output_with(
        &self,
        target: &OsStr,
        job: &JobControl,
        render: &mut dyn FnMut(&mut dyn Write) -> Result<(), ApiError>,
    ) -> Result<(Spec, u64), ApiError> {
        run_guarded(|| {
            let spec = self.write_spec(target)?;
            let bytes = self.stream_into(&*self.router(), &spec, job, None, render)?;
            Ok((spec, bytes))
        })
    }

    /// The body of [`Service::write_output_with`] for a parsed spec; `durability` `None` picks by whether the target
    /// exists.
    pub(crate) fn stream_into(
        &self,
        router: &dyn Vfs,
        spec: &Spec,
        job: &JobControl,
        durability: Option<Durability>,
        render: &mut dyn FnMut(&mut dyn Write) -> Result<(), ApiError>,
    ) -> Result<u64, ApiError> {
        let durability =
            durability.unwrap_or_else(|| match router.stat(spec, hikasa_vfs::FollowLinks::No) {
                Ok(_) => Durability::FileOnly,
                Err(_) => Durability::None,
            });
        let options = WriteOptions {
            create_parents: false,
            overwrite: true,
            replace_link: false,
            durability,
        };
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        let mut writer = router
            .open_write(spec, &options)
            .map_err(ApiError::from_vfs)?;
        let written = {
            let mut buffered = std::io::BufWriter::with_capacity(REPORT_BUFFER, &mut writer);
            let mut cancel = CancelWriter::new(&mut buffered, &job.cancel);
            render(&mut cancel)?;
            let written = cancel.written();
            buffered.flush().map_err(|e| write_error(spec, e))?;
            written
        };
        if job.cancel.is_cancelled() {
            return Err(ApiError::Cancelled);
        }
        writer.commit().map_err(ApiError::from_vfs)?;
        Ok(written)
    }

    /// Writes report output to a spec atomically (`--output`).
    ///
    /// The target is replaced through the VFS's atomic writer (temporary file, fsync, rename): on any error the
    /// target is untouched. Its folder must exist.
    pub fn write_output(&self, target: &OsStr, bytes: &[u8]) -> Result<Spec, ApiError> {
        run_guarded(|| {
            // `-` as an output target is refused by the VFS; it never reads standard input. A target inside an
            // archive is a member: its commit is a rebuild (D-148).
            let spec = self.write_spec(target)?;
            let options = WriteOptions {
                create_parents: false,
                overwrite: true,
                replace_link: false,
                ..WriteOptions::default()
            };
            let mut writer = self
                .vfs()
                .open_write(&spec, &options)
                .map_err(ApiError::from_vfs)?;
            writer.write_all(bytes).map_err(|e| {
                ApiError::from_vfs(VfsError::from_io(spec.canonical(), VfsOp::Write, e))
            })?;
            writer.commit().map_err(ApiError::from_vfs)?;
            Ok(spec)
        })
    }
}

/// A write error of a report into `spec`: a VFS write error naming it.
pub(crate) fn write_error(spec: &Spec, error: std::io::Error) -> ApiError {
    ApiError::from_vfs(VfsError::from_io(spec.canonical(), VfsOp::Write, error))
}

/// The error of a renderer writing into `spec` (`None`: the caller's stdout): a [`CancelWriter`]'s stop is
/// `Cancelled`, a write error names the target, anything else is the report's own error.
pub(crate) fn render_error(spec: Option<&Spec>, error: hikasa_report::ReportError) -> ApiError {
    if error.is_cancelled() {
        return ApiError::Cancelled;
    }
    match (spec, error) {
        (Some(spec), hikasa_report::ReportError::Io(e)) => write_error(spec, e),
        (None, hikasa_report::ReportError::Io(e)) => ApiError::output(e),
        (_, other) => ApiError::Report(other),
    }
}

/// `--lines [left:|right:]<a>-<b>` (D-209): left by default.
fn line_scope(text: &str) -> Result<LineScope, ApiError> {
    let trimmed = text.trim();
    let (side, range) = match trimmed.split_once(':') {
        Some((side, range)) => {
            let side = match side.trim().to_ascii_lowercase().as_str() {
                "left" | "lt" => Side::Left,
                "right" | "rt" => Side::Right,
                _ => {
                    return Err(ApiError::InvalidArgument(format!(
                        "invalid side {side:?} in --lines {text:?} (valid: left, right)"
                    )));
                }
            };
            (side, range)
        }
        None => (Side::Left, trimmed),
    };
    let range = parse_line_range(range)?;
    Ok(LineScope {
        side,
        first: *range.start(),
        last: *range.end(),
    })
}

/// `SOURCE_DATE_EPOCH` as a time in UTC: decimal digits only (no sign, no space, as reproducible-builds.org defines
/// it), in jiff's range; else `None`.
pub(crate) fn source_date_epoch(raw: &str) -> Option<jiff::Zoned> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let seconds: i64 = raw.parse().ok()?;
    let at = jiff::Timestamp::from_second(seconds).ok()?;
    Some(at.to_zoned(jiff::tz::TimeZone::UTC))
}

/// A writer that drops every line starting with `# ` (the folder list's summary line), streaming.
struct DropSummaryLines<'a> {
    out: &'a mut dyn Write,
    at_line_start: bool,
    state: LineState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    /// Copying.
    Normal,
    /// A `#` at a line start is held back until the next byte decides.
    Hash,
    /// Inside a dropped line (up to and including its newline).
    Dropping,
}

impl<'a> DropSummaryLines<'a> {
    fn new(out: &'a mut dyn Write) -> DropSummaryLines<'a> {
        DropSummaryLines {
            out,
            at_line_start: true,
            state: LineState::Normal,
        }
    }

    /// Writes a held-back `#` and flushes.
    fn finish(self) -> std::io::Result<()> {
        if self.state == LineState::Hash {
            self.out.write_all(b"#")?;
        }
        self.out.flush()
    }
}

impl Write for DropSummaryLines<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut kept = Vec::with_capacity(buf.len());
        for &b in buf {
            match self.state {
                LineState::Normal => {
                    if self.at_line_start && b == b'#' {
                        self.state = LineState::Hash;
                    } else {
                        kept.push(b);
                        self.at_line_start = b == b'\n';
                    }
                }
                LineState::Hash => {
                    if b == b' ' {
                        self.state = LineState::Dropping;
                    } else {
                        kept.push(b'#');
                        kept.push(b);
                        self.at_line_start = b == b'\n';
                        self.state = LineState::Normal;
                    }
                }
                LineState::Dropping => {
                    if b == b'\n' {
                        self.state = LineState::Normal;
                        self.at_line_start = true;
                    }
                }
            }
        }
        self.out.write_all(&kept)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lines_are_dropped() {
        let mut out = Vec::new();
        let mut filter = DropSummaryLines::new(&mut out);
        for piece in [
            &b"=  a\n!  b/"[..],
            b"\n#",
            b" 1 same, 0 left newer\n",
            b"#x\n",
        ] {
            filter.write_all(piece).unwrap();
        }
        filter.finish().unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "=  a\n!  b/\n#x\n");
    }
}
