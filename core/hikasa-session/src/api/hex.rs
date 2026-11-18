//! Hex Compare in the Service (§1.12.22, D-345; W12c): [`HexCompareRequest`] with the user's words ([`HexRuleWords`],
//! [`HexLayoutWords`]), [`Service::hex_open`], [`Service::hex_align`], [`Service::hex_compare`] (open, align, the
//! verdict of D-331, the stamps re-checked), [`Service::hex_dump`] and [`Service::hex_find`] (the files of `text
//! find`'s walk, D-334), the human output ([`HexCompareResponse::human_lines`]) and the Hex format's load conversion
//! (D-336: the converted output in the temp cache).
//!
//! Every word of hex is parsed here, by the parsers of `hikasa-hex` (check 7): the adapters pass the words as typed.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{
    CaseSensitivity, ContentKind, ErrorReport, ExitCode, MaskOptions, NameFilter, Pair, Side, Spec,
    Warning, WarningCode,
};
use hikasa_formats::{Conversion, ExternalConversion, FormatKind, run_external};
use hikasa_hex::vocab as words;
use hikasa_hex::{
    AlignMode, Converted, DisplayFilter, HexAlignOptions, HexAlignment, HexConversion, HexDocument,
    HexError, HexFinder, HexLayout, HexLoadOptions, HexPattern, HexWarning, HexWarningCode, Radix,
    TextEncoding, Values, nav, nav::AddressQuery, values,
};
use hikasa_report::json::{
    FormatJson, HexCompareJson, HexDumpJson, HexDumpRowJson, HexFindCountsJson, HexFindFileJson,
    HexFindJson, HexLayoutJson, HexQueryJson, HexSideJson, HexSkippedJson, ValuesBlockJson,
    hex_dump_rows,
};
use hikasa_report::{HexSide, HexSource, hex_compare_json, hex_summary_line, section_line};
use hikasa_text::find::files::{FileWalk, FindArchives, WalkItem, WalkScope};
use hikasa_vfs::{EntryKind, Vfs, VfsSource, compare_sources};
use serde::{Deserialize, Serialize};

use super::compare::load_failed;
use super::table::format_json;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

// ---- the words -----------------------------------------------------------------------------------------------------

fn is_none<T>(v: &Option<T>) -> bool {
    v.is_none()
}

/// The rule words of a hex comparison as typed (§1.12.23; a session's `[rules.hex]`, D-349).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexRuleWords {
    /// `--alignment none|fast|complete` (default `fast`).
    #[serde(skip_serializing_if = "is_none")]
    pub alignment: Option<String>,
    /// `--block <bytes>`: Fast's average block, a power of two from 1 KiB to 1 MiB (`8k`, `8192`).
    #[serde(skip_serializing_if = "is_none")]
    pub block: Option<String>,
    /// `--skew <bytes>`.
    #[serde(skip_serializing_if = "is_none")]
    pub skew: Option<String>,
    /// `--window <bytes>`: 4 KiB…16 MiB.
    #[serde(skip_serializing_if = "is_none")]
    pub window: Option<String>,
    /// `--budget <MiB>`: 1…1,048,576.
    #[serde(skip_serializing_if = "is_none")]
    pub budget_mib: Option<String>,
    /// `--unlocked-max <MiB>` (HX-002): 0…1,024.
    #[serde(skip_serializing_if = "is_none")]
    pub unlocked_max_mib: Option<String>,
}

impl HexRuleWords {
    /// True when no word is given.
    pub fn is_empty(&self) -> bool {
        *self == HexRuleWords::default()
    }
}

/// The layout and display words of a hex comparison as typed (§1.12.23; a session's `[view]` and `[display]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexLayoutWords {
    /// `--bytes-per-line <n>`: 1…256.
    #[serde(skip_serializing_if = "is_none")]
    pub bytes_per_line: Option<String>,
    /// `--group 1|2|4|8`.
    #[serde(skip_serializing_if = "is_none")]
    pub group: Option<String>,
    /// `--chars ascii|latin1|none`.
    #[serde(skip_serializing_if = "is_none")]
    pub chars: Option<String>,
    /// `--address hex|decimal`.
    #[serde(skip_serializing_if = "is_none")]
    pub address: Option<String>,
    /// `--context <n>`: 0…64.
    #[serde(skip_serializing_if = "is_none")]
    pub context: Option<String>,
    /// `--display all|differences|same|none`.
    #[serde(skip_serializing_if = "is_none")]
    pub display: Option<String>,
}

impl HexLayoutWords {
    /// True when no word is given.
    pub fn is_empty(&self) -> bool {
        *self == HexLayoutWords::default()
    }

    /// True when a word that shapes rows is given (`--bytes-per-line`, `--group`, `--chars`, `--address`,
    /// `--context`): the human output and the reports use them, `--json` alone does not.
    pub fn shapes_rows(&self) -> bool {
        self.bytes_per_line.is_some()
            || self.group.is_some()
            || self.chars.is_some()
            || self.address.is_some()
            || self.context.is_some()
    }
}

/// The words of a Hex Compare view in an `OpenView` (D-350): the rule and layout words and the per-side formats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexWords {
    /// The rule words.
    pub rules: HexRuleWords,
    /// The layout words.
    pub layout: HexLayoutWords,
    /// `--left-format`, `--right-format`.
    pub formats: [Option<String>; 2],
}

/// The parsed settings of a hex request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexSettings {
    /// The alignment.
    pub align: HexAlignOptions,
    /// The loads.
    pub load: HexLoadOptions,
    /// The rows' layout.
    pub layout: HexLayout,
    /// The display (`None`: `none`).
    pub display: Option<DisplayFilter>,
    /// Context rows.
    pub context: u8,
}

fn vocab(e: words::VocabError) -> ApiError {
    ApiError::InvalidArgument(e.to_string())
}

/// A size word in bytes (`8k`, `1MiB`, `8192`) checked by `check` (which takes the number as decimal text).
fn size_word<T>(
    word: &str,
    flag: &str,
    check: impl FnOnce(&str) -> Result<T, words::VocabError>,
) -> Result<T, ApiError> {
    let n = super::vocab::parse_byte_size(word, flag)?;
    // The refusal names the word as typed (`3k`), not the number it stands for (F11).
    check(&n.to_string()).map_err(|mut e| {
        e.word = word.trim().to_owned();
        vocab(e)
    })
}

/// The display word of hex: `all`, `differences`, `same`, `none`.
pub fn parse_hex_display(word: &str) -> Result<Option<DisplayFilter>, ApiError> {
    if word.trim().eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    DisplayFilter::parse(word).map(Some).map_err(|_| {
        ApiError::InvalidArgument(format!(
            "invalid display {word:?}: expected all, differences, same, none"
        ))
    })
}

/// Whether the human output of `hex compare` with the display word `word` (`None`: the default, `differences`) prints
/// rows (`all`, `same`), which the layout words and `context` shape, or sections (`differences`) or nothing (`none`).
pub fn hex_display_shows_rows(word: Option<&str>) -> Result<bool, ApiError> {
    Ok(match word {
        None => false,
        Some(word) => {
            parse_hex_display(word)?.is_some_and(|filter| filter != DisplayFilter::Differences)
        }
    })
}

/// Parses the rule words onto the defaults.
pub fn hex_rules_of(r: &HexRuleWords) -> Result<(HexAlignOptions, HexLoadOptions), ApiError> {
    let mut align = HexAlignOptions::default();
    if let Some(w) = &r.alignment {
        align.mode = AlignMode::parse(w).map_err(vocab)?;
    }
    if let Some(w) = &r.block {
        align.block = size_word(w, "--block", words::parse_block)?;
    }
    if let Some(w) = &r.skew {
        align.skew = size_word(w, "--skew", words::parse_skew)?;
    }
    if let Some(w) = &r.window {
        align.window = size_word(w, "--window", words::parse_window)?;
    }
    if let Some(w) = &r.budget_mib {
        align.budget_mib = words::parse_budget_mib(w).map_err(vocab)?;
    }
    let mut load = HexLoadOptions::default();
    if let Some(w) = &r.unlocked_max_mib {
        let mib = w
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|n| *n <= 1024)
            .ok_or_else(|| {
                ApiError::InvalidArgument(format!(
                    "invalid unlocked maximum {w:?}: expected a number of MiB from 0 to 1024"
                ))
            })?;
        load.unlocked_max = mib << 20;
    }
    Ok((align, load))
}

/// Parses the layout words onto the defaults (`display_default` without `--display`).
pub fn hex_layout_of(
    l: &HexLayoutWords,
    display_default: Option<DisplayFilter>,
) -> Result<(HexLayout, Option<DisplayFilter>, u8), ApiError> {
    let mut layout = HexLayout::default();
    if let Some(w) = &l.bytes_per_line {
        layout.bytes_per_line = words::parse_bytes_per_line(w).map_err(vocab)?;
    }
    if let Some(w) = &l.group {
        layout.group = words::parse_group(w).map_err(vocab)?;
    }
    if let Some(w) = &l.chars {
        layout.chars = words::CharSet::parse(w).map_err(vocab)?;
    }
    if let Some(w) = &l.address {
        layout.address = Radix::parse(w).map_err(vocab)?;
    }
    let context = match &l.context {
        Some(w) => words::parse_context(w).map_err(vocab)?,
        None => 0,
    };
    let display = match &l.display {
        Some(w) => parse_hex_display(w)?,
        None => display_default,
    };
    Ok((layout, display, context))
}

/// `--max-sections <n>` (0: all; default 20).
pub fn parse_max_sections_listed(word: Option<&str>) -> Result<u64, ApiError> {
    match word {
        None => Ok(HexCompareRequest::DEFAULT_MAX_SECTIONS),
        Some(w) => w.trim().parse::<u64>().map_err(|_| {
            ApiError::InvalidArgument(format!(
                "invalid --max-sections {w:?}: expected a number (0: all)"
            ))
        }),
    }
}

/// `--preview <bytes>` (0…4,096; default 16).
pub fn parse_preview(word: Option<&str>) -> Result<u32, ApiError> {
    match word {
        None => Ok(HexCompareRequest::DEFAULT_PREVIEW),
        Some(w) => {
            let n = super::vocab::parse_byte_size(w, "--preview")?;
            u32::try_from(n).ok().filter(|n| *n <= 4096).ok_or_else(|| {
                ApiError::InvalidArgument(format!(
                    "invalid --preview {w:?}: expected 0 to 4096 bytes"
                ))
            })
        }
    }
}

// ---- the request and the response ------------------------------------------------------------------------------

/// `hikasa hex compare` (§1.12.23).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HexCompareRequest {
    /// Left spec as typed.
    pub left: OsString,
    /// Right spec as typed.
    pub right: OsString,
    /// `--format <id|name>`: the Hex format of both sides.
    pub format: Option<String>,
    /// `--left-format`, `--right-format` (D-281).
    pub formats: Pair<Option<String>>,
    /// The rule words.
    pub rules: HexRuleWords,
    /// The layout words.
    pub layout: HexLayoutWords,
    /// `--max-sections <n>` (0: all; default 20).
    pub max_sections: Option<String>,
    /// `--preview <bytes>` (default 16).
    pub preview: Option<String>,
    /// FF-002: formats on or off for this comparison.
    pub format_overrides: std::collections::BTreeMap<String, bool>,
    /// `--verbose`.
    pub verbose: bool,
}

impl HexCompareRequest {
    /// Sections printed by default.
    pub const DEFAULT_MAX_SECTIONS: u64 = 20;
    /// Preview bytes a side by default.
    pub const DEFAULT_PREVIEW: u32 = 16;

    /// A hex comparison of two specs that were already parsed.
    pub fn for_specs(left: &Spec, right: &Spec) -> HexCompareRequest {
        HexCompareRequest {
            left: super::spec_os_string(left),
            right: super::spec_os_string(right),
            ..HexCompareRequest::default()
        }
    }

    /// The words of a view (D-350).
    pub fn words(&self) -> HexWords {
        HexWords {
            rules: self.rules.clone(),
            layout: self.layout.clone(),
            formats: [self.formats.left.clone(), self.formats.right.clone()],
        }
    }
}

/// The Hex format a side was read with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexFormatChoice {
    /// The Hikasa format of the Hex kind (`None`: none matched; the raw bytes).
    pub format: Option<FormatJson>,
    /// Its load conversion.
    pub conversion: Option<ExternalConversion>,
}

/// Both documents of a hex comparison, opened ([`Service::hex_open`]).
#[derive(Debug, Clone)]
pub struct HexOpened {
    /// The specs.
    pub specs: Pair<Spec>,
    /// The documents.
    pub documents: Pair<Arc<HexDocument>>,
    /// The formats.
    pub formats: Pair<HexFormatChoice>,
    /// What the reports and the JSON show of each side.
    pub sides: Pair<HexSide>,
}

impl HexOpened {
    /// True when a side was converted.
    pub fn converted(&self) -> bool {
        self.sides.left.converted || self.sides.right.converted
    }
}

/// The result of [`Service::hex_compare`].
#[derive(Debug, Clone)]
pub struct HexCompareResponse {
    /// Both documents.
    pub opened: HexOpened,
    /// Their alignment.
    pub alignment: Arc<HexAlignment>,
    /// The alignment options.
    pub options: HexAlignOptions,
    /// The verdict (D-331).
    pub kind: ContentKind,
    /// The exit code (1, 2, 11, 13).
    pub exit_code: ExitCode,
    /// The rows' layout.
    pub layout: HexLayout,
    /// The display (`None`: `none`).
    pub display: Option<DisplayFilter>,
    /// Context rows.
    pub context: u8,
    /// Sections listed (0: all).
    pub max_sections: u64,
    /// Preview bytes a side.
    pub preview: u32,
    /// An input changed during the job.
    pub stale: bool,
    /// Warnings.
    pub warnings: Vec<Warning>,
}

impl HexCompareResponse {
    /// The input of the reports and of the JSON document.
    pub fn source(&self) -> HexSource<'_> {
        HexSource {
            documents: Pair::new(&*self.opened.documents.left, &*self.opened.documents.right),
            alignment: &self.alignment,
            options: &self.options,
            sides: Pair::new(&self.opened.sides.left, &self.opened.sides.right),
            kind: self.kind,
            stale: self.stale,
            layout: self.layout,
            max_sections: self.max_sections,
            preview: self.preview,
            warnings: &self.warnings,
        }
    }

    /// `hikasa.hex_compare.v1`.
    ///
    /// # Errors
    ///
    /// A read error of a preview.
    pub fn json(&self) -> Result<HexCompareJson, ApiError> {
        Ok(hex_compare_json(&self.source())?)
    }

    /// The human output (§1.12.23): with `differences` one line per listed section, with `all` or `same` the rows
    /// under that filter (the side-by-side text of the report without its header), with `none` nothing; then the
    /// summary line. The caller prints the verdict line.
    ///
    /// # Errors
    ///
    /// A read error.
    pub fn human_lines(&self) -> Result<Vec<String>, ApiError> {
        let src = self.source();
        let mut out = Vec::new();
        match self.display {
            Some(DisplayFilter::Differences) => {
                for s in src.listed_sections() {
                    out.push(section_line(&src, &s)?);
                }
                let stored = self.alignment.sections.len() as u64;
                let listed = out.len() as u64;
                if listed < self.alignment.stats.sections {
                    let more = self.alignment.stats.sections - listed;
                    out.push(format!(
                        "... {more} more {} (--max-sections 0 lists {})",
                        if more == 1 { "section" } else { "sections" },
                        if stored < self.alignment.stats.sections {
                            "the stored ones"
                        } else {
                            "all"
                        }
                    ));
                }
            }
            Some(filter) => {
                let opts = hikasa_report::HexReportOptions {
                    layout: hikasa_report::HexReportLayout::SideBySide,
                    display: Some(filter),
                    context: self.context,
                    addresses: true,
                    format: hikasa_report::OutputFormat::Text { ansi: false },
                };
                let meta = hikasa_report::ReportMeta {
                    title: None,
                    generated_at: None,
                    generator: String::new(),
                    mode: None,
                    filters: None,
                    errors: Pair::new(None, None),
                    folder_link: None,
                };
                let mut bytes = Vec::new();
                hikasa_report::render_hex_report(
                    &src,
                    &opts,
                    &meta,
                    &hikasa_report::HtmlStyle::default(),
                    &mut bytes,
                )?;
                let text = String::from_utf8_lossy(&bytes);
                // The rows: after the header's blank line, up to the blank line before the summary line.
                let mut lines = text.lines().skip_while(|l| !l.is_empty()).skip(1);
                for line in lines.by_ref() {
                    if line.is_empty() {
                        break;
                    }
                    out.push(line.to_owned());
                }
            }
            None => {}
        }
        out.push(hex_summary_line(&self.alignment.stats, self.alignment.mode));
        Ok(out)
    }
}

/// A hex warning as a [`Warning`].
pub fn hex_warning(w: &HexWarning) -> Warning {
    Warning {
        code: match w.code {
            HexWarningCode::AlignmentDegraded => WarningCode::AlignmentDegraded,
            HexWarningCode::SectionsTruncated => WarningCode::SectionsTruncated,
            _ => WarningCode::InputChanged,
        },
        message: w.message.clone(),
        spec: w.spec.clone(),
    }
}

/// The member of a pair on `side`.
fn of<T>(pair: &Pair<T>, side: Side) -> &T {
    match side {
        Side::Left => &pair.left,
        _ => &pair.right,
    }
}

/// A hex error as an API error: a cancellation is `Cancelled`.
fn hex_error(e: HexError) -> ApiError {
    if e.is_cancelled() {
        ApiError::Cancelled
    } else {
        ApiError::Hex(e)
    }
}

// ---- the load conversion (D-336) ---------------------------------------------------------------------------------

/// A converted output in a private folder of the temp folder (W12c review fixes, F2): the folder is made with a fresh
/// name and mode 0700, the file in it exclusively (`O_EXCL`) with mode 0600; both are removed when the document that
/// reads it is dropped (the file first).
struct ConvertedOutput {
    _file: tempfile::TempPath,
    _dir: tempfile::TempDir,
}

/// Writes `output` to a new file in a new private folder below `temp`.
fn write_converted(
    temp: &std::path::Path,
    output: &[u8],
) -> std::io::Result<(PathBuf, ConvertedOutput)> {
    use std::io::Write;
    let mut builder = tempfile::Builder::new();
    builder.prefix("hikasa-hex-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let dir = builder.tempdir_in(temp)?;
    let mut file = tempfile::Builder::new()
        .prefix("converted-")
        .suffix(".out")
        .tempfile_in(dir.path())?;
    file.write_all(output)?;
    file.flush()?;
    // The handle is closed (Windows readers open the path again); the path removes the file when dropped.
    let file = file.into_temp_path();
    let path = file.to_path_buf();
    Ok((
        path,
        ConvertedOutput {
            _file: file,
            _dir: dir,
        },
    ))
}

/// The largest input a load conversion reads (512 MiB).
const CONVERT_MAX: u64 = 512 << 20;

/// A Hex format's External load conversion: the input read through the VFS, the program run by the Service's runner
/// (`run_external`), its output written to the temp cache, which the document then reads (`TempCache`).
struct ExternalHex<'a> {
    service: &'a Service,
    conversion: &'a ExternalConversion,
}

impl HexConversion for ExternalHex<'_> {
    fn convert(
        &self,
        vfs: &dyn Vfs,
        spec: &Spec,
        ctl: &hikasa_core::Ctl<'_>,
    ) -> Result<Converted, HexError> {
        let display = spec.canonical();
        let mut reader = vfs
            .open_read_with(spec, ctl.cancel)
            .map_err(|e| HexError::from_vfs(&display, e))?;
        let mut input = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(&mut reader, CONVERT_MAX + 1),
            &mut input,
        )
        .map_err(|e| {
            HexError::from_vfs(
                &display,
                hikasa_vfs::VfsError::from_io(display.clone(), hikasa_vfs::VfsOp::Read, e),
            )
        })?;
        if input.len() as u64 > CONVERT_MAX {
            return Err(HexError::Convert {
                spec: display,
                message: format!("larger than the conversion limit of {CONVERT_MAX} bytes"),
                kind: hikasa_core::ErrorKind::TooLarge,
            });
        }
        let (temp, runner) = self.service.external();
        let output = run_external(self.conversion, &input, &display, temp, runner, ctl.cancel)
            .map_err(|e| match e {
                hikasa_formats::ConvertError::Cancelled => HexError::Cancelled,
                other => HexError::Convert {
                    spec: display.clone(),
                    kind: hikasa_core::Classify::kind(&other),
                    message: other.to_string(),
                },
            })?;
        let (path, guard) = write_converted(temp, &output).map_err(|e| HexError::Convert {
            spec: display.clone(),
            kind: hikasa_core::io_error_kind(&e),
            message: format!("cannot write the converted output: {e}"),
        })?;
        let local = hikasa_core::LocalPath::new(path).map_err(|e| HexError::Convert {
            spec: display.clone(),
            kind: hikasa_core::ErrorKind::Internal,
            message: e.to_string(),
        })?;
        Ok(Converted {
            spec: Spec::Local(local),
            keep: Some(Box::new(guard)),
        })
    }
}

// ---- the Service ---------------------------------------------------------------------------------------------------

/// `--format` with a side's own format is refused (D-281).
fn format_combination(
    format: &Option<String>,
    formats: &Pair<Option<String>>,
) -> Result<(), ApiError> {
    if format.is_some() && (formats.left.is_some() || formats.right.is_some()) {
        return Err(ApiError::InvalidArgument(
            "--format sets the format of both sides; it cannot be combined with --left-format or \
             --right-format"
                .to_owned(),
        ));
    }
    Ok(())
}

impl Service {
    /// The words of a hex request checked as [`Service::hex_compare`] checks them before anything is read: the rule
    /// and layout words, `--max-sections`, `--preview` and the forced formats (known, of the Hex kind). What `hex compare
    /// --gui` checks locally (D-350).
    pub fn check_hex_words(&self, req: &HexCompareRequest) -> Result<(), ApiError> {
        run_guarded(|| {
            self.hex_settings(req)?;
            parse_max_sections_listed(req.max_sections.as_deref())?;
            parse_preview(req.preview.as_deref())?;
            format_combination(&req.format, &req.formats)?;
            for word in [&req.format, &req.formats.left, &req.formats.right]
                .into_iter()
                .flatten()
            {
                self.hex_format_forced(word)?;
            }
            Ok(())
        })
    }

    /// The settings of a request (every word parsed).
    pub fn hex_settings(&self, req: &HexCompareRequest) -> Result<HexSettings, ApiError> {
        let (align, load) = hex_rules_of(&req.rules)?;
        let (layout, display, context) =
            hex_layout_of(&req.layout, Some(DisplayFilter::Differences))?;
        Ok(HexSettings {
            align,
            load,
            layout,
            display,
            context,
        })
    }

    /// A forced format: known and of the Hex kind.
    fn hex_format_forced(&self, word: &str) -> Result<hikasa_formats::Detection, ApiError> {
        let detection = self.registry().force(word)?;
        if detection.format.kind() != FormatKind::Hex {
            return Err(ApiError::InvalidArgument(format!(
                "format `{}` is a {} format: hex compare takes a format of the hex kind",
                detection.format.name(),
                detection.format.kind()
            )));
        }
        Ok(detection)
    }

    /// The Hex format of a side (D-336): its own (`--left-format`/`--right-format`), else `--format`, else the
    /// detection of its name among the enabled Hex formats (D-10); none matched: the raw bytes.
    fn hex_format(
        &self,
        spec: &Spec,
        own: Option<&str>,
        both: Option<&str>,
        overrides: &hikasa_formats::EnabledOverrides,
    ) -> Result<HexFormatChoice, ApiError> {
        let detection = match own.or(both) {
            Some(word) => Some(self.hex_format_forced(word)?),
            None => self
                .detection_name(spec)
                .and_then(|name| self.registry().detect(&name, overrides).ok())
                .filter(|d| d.format.kind() == FormatKind::Hex),
        };
        Ok(match detection {
            Some(d) => HexFormatChoice {
                format: Some(format_json(&d)),
                conversion: match d.format.conversion() {
                    Conversion::External(x) => Some(x.clone()),
                    _ => None,
                },
            },
            None => HexFormatChoice {
                format: None,
                conversion: None,
            },
        })
    }

    /// Opens both hex documents (§1.12.22: stat, open, convert): the words parsed first (a bad word is exit 100
    /// before anything is read); each side's Hex format; both documents opened in parallel on the pool.
    pub fn hex_open(
        &self,
        req: &HexCompareRequest,
        job: &JobControl,
    ) -> Result<HexOpened, ApiError> {
        run_guarded(|| {
            let settings = self.hex_settings(req)?;
            self.hex_open_with(req, &settings.load, job)
        })
    }

    fn hex_open_with(
        &self,
        req: &HexCompareRequest,
        load: &HexLoadOptions,
        job: &JobControl,
    ) -> Result<HexOpened, ApiError> {
        format_combination(&req.format, &req.formats)?;
        let left = self.parse_input(None, &req.left, &job.cancel)?;
        let right = self.parse_input(None, &req.right, &job.cancel)?;
        let specs = Pair::new(left, right);
        let mut entries = Vec::with_capacity(2);
        for side in [Side::Left, Side::Right] {
            let spec = of(&specs, side);
            let entry = self.stat_input(side, spec, job)?;
            if entry.kind == EntryKind::Dir {
                return Err(ApiError::InvalidArgument(format!(
                    "{}: hex compare compares files; use `folder compare` for folders",
                    spec.canonical()
                )));
            }
            entries.push(entry);
        }
        let mut warnings = Vec::new();
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let formats = Pair::new(
            self.hex_format(
                &specs.left,
                req.formats.left.as_deref(),
                req.format.as_deref(),
                &overrides,
            )?,
            self.hex_format(
                &specs.right,
                req.formats.right.as_deref(),
                req.format.as_deref(),
                &overrides,
            )?,
        );
        let router = self.router();
        let ctl = job.ctl();
        let open = |side: Side| -> Result<HexDocument, ApiError> {
            let spec = of(&specs, side);
            let opened = match &of(&formats, side).conversion {
                Some(conversion) => HexDocument::open_converted(
                    &*router,
                    spec,
                    &ExternalHex {
                        service: self,
                        conversion,
                    },
                    load,
                    &ctl,
                ),
                None => HexDocument::open(&*router, spec, load, &ctl),
            };
            opened.map_err(|e| match e {
                HexError::Io { source, .. } => load_failed(side, spec, source),
                other => hex_error(other),
            })
        };
        let (l, r) = self
            .pool()
            .install(|| rayon::join(|| open(Side::Left), || open(Side::Right)));
        let documents = Pair::new(Arc::new(l?), Arc::new(r?));
        let side_of = |side: Side, entry: &hikasa_vfs::Entry| {
            let doc = of(&documents, side);
            let info = doc.info();
            HexSide {
                spec: of(&specs, side).canonical(),
                size: doc.len(),
                mtime: entry.mtime.and_then(|t| {
                    jiff::Timestamp::new(t.unix_seconds(), t.subsec_nanos() as i32).ok()
                }),
                format: of(&formats, side).format.clone(),
                converted: info.converted,
                load: info.load,
            }
        };
        let sides = Pair::new(
            side_of(Side::Left, &entries[0]),
            side_of(Side::Right, &entries[1]),
        );
        Ok(HexOpened {
            specs,
            documents,
            formats,
            sides,
        })
    }

    /// Aligns two opened documents (§1.12.22) on the pool; the visitor's stream options are the Service's.
    pub fn hex_align(
        &self,
        opened: &HexOpened,
        opts: &HexAlignOptions,
        job: &JobControl,
    ) -> Result<Arc<HexAlignment>, ApiError> {
        run_guarded(|| {
            let mut opts = *opts;
            opts.stream = self.stream_options();
            let ctl = job.ctl();
            let aligned = self.pool().install(|| {
                hikasa_hex::align(&opened.documents.left, &opened.documents.right, &opts, &ctl)
            });
            aligned.map(Arc::new).map_err(hex_error)
        })
    }

    /// Hex compare (§1.12.22): the words parsed; both documents opened; aligned; the verdict of D-331 (a converted side:
    /// the raw inputs' bytes first, then the converted outputs'); each document's stamp re-checked (`input_changed`,
    /// `stale`).
    pub fn hex_compare(
        &self,
        req: &HexCompareRequest,
        job: &JobControl,
    ) -> Result<HexCompareResponse, ApiError> {
        run_guarded(|| self.hex_compare_inner(req, job))
    }

    fn hex_compare_inner(
        &self,
        req: &HexCompareRequest,
        job: &JobControl,
    ) -> Result<HexCompareResponse, ApiError> {
        let settings = self.hex_settings(req)?;
        let max_sections = parse_max_sections_listed(req.max_sections.as_deref())?;
        let preview = parse_preview(req.preview.as_deref())?;
        let opened = self.hex_open_with(req, &settings.load, job)?;
        let alignment = self.hex_align(&opened, &settings.align, job)?;
        let verdict = if opened.converted() {
            let router = self.router();
            let source = |side: Side| -> Result<VfsSource<'_>, ApiError> {
                let spec = of(&opened.specs, side);
                let entry = self.stat_input(side, spec, job)?;
                Ok(VfsSource::new(&*router, spec.clone(), entry))
            };
            let (l, r) = (source(Side::Left)?, source(Side::Right)?);
            let raw_same =
                compare_sources(&l, &r, &self.stream_options(), &job.ctl()).map_err(|e| {
                    if e.is_cancelled() {
                        ApiError::Cancelled
                    } else {
                        ApiError::from_vfs(e)
                    }
                })? == hikasa_vfs::BinaryOutcome::Same;
            hikasa_hex::converted_verdict(raw_same, &alignment)
        } else {
            hikasa_hex::verdict(&alignment)
        };
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        let mut warnings: Vec<Warning> = alignment.warnings.iter().map(hex_warning).collect();
        let router = self.router();
        let mut stale = false;
        for doc in [&opened.documents.left, &opened.documents.right] {
            if let Err(changed) = doc.check_unchanged(&*router) {
                stale = true;
                warnings.push(hex_warning(&changed.warning()));
            }
        }
        Ok(HexCompareResponse {
            opened,
            alignment,
            options: settings.align,
            kind: verdict.kind,
            exit_code: verdict.exit_code,
            layout: settings.layout,
            display: settings.display,
            context: settings.context,
            max_sections,
            preview,
            stale,
            warnings,
        })
    }

    /// `hex dump` (§1.12.23): `length` bytes of one side from `offset` (decimal by default; every Go To spelling), the
    /// rows of `layout`, the value panel at `offset` with `values`.
    pub fn hex_dump(
        &self,
        req: &HexDumpRequest,
        job: &JobControl,
    ) -> Result<HexDumpResponse, ApiError> {
        run_guarded(|| self.hex_dump_inner(req, job))
    }

    fn hex_dump_inner(
        &self,
        req: &HexDumpRequest,
        job: &JobControl,
    ) -> Result<HexDumpResponse, ApiError> {
        let (layout, _, _) = hex_layout_of(&req.layout, None)?;
        let offset_query = match &req.offset {
            Some(w) => Some(nav::parse_address(w, Radix::Decimal).map_err(vocab)?),
            None => None,
        };
        if let Some(AddressQuery::Relative(d)) = offset_query
            && d < 0
        {
            return Err(ApiError::InvalidArgument(format!(
                "--offset {}: a dump starts at an offset from the start of the file (0 or more, or `end`)",
                req.offset.as_deref().unwrap_or_default().trim()
            )));
        }
        let length = match &req.length {
            Some(w) => {
                let n = super::vocab::parse_byte_size(w, "--length")?;
                if n > HexDumpRequest::MAX_LENGTH {
                    return Err(ApiError::InvalidArgument(format!(
                        "invalid --length {w:?}: at most 16 MiB"
                    )));
                }
                n
            }
            None => HexDumpRequest::DEFAULT_LENGTH,
        };
        if let Some(word) = &req.format {
            self.hex_format_forced(word)?;
        }
        let compare = HexCompareRequest {
            left: req.spec.clone(),
            right: req.spec.clone(),
            format: req.format.clone(),
            ..HexCompareRequest::default()
        };
        let spec = self.parse_input(None, &req.spec, &job.cancel)?;
        // A dump has one input, not a left one: its read errors name the spec alone (F11).
        let one_input = |e: ApiError| match e {
            ApiError::LoadFailed { source, .. } => ApiError::from_vfs(source),
            other => other,
        };
        let entry = self.stat_input(Side::Left, &spec, job).map_err(one_input)?;
        if entry.kind == EntryKind::Dir {
            return Err(ApiError::InvalidArgument(format!(
                "{}: hex dump reads a file",
                spec.canonical()
            )));
        }
        let mut warnings = Vec::new();
        let overrides = self.enabled_overrides(&compare.format_overrides, &mut warnings);
        let format = self.hex_format(&spec, None, req.format.as_deref(), &overrides)?;
        let router = self.router();
        let ctl = job.ctl();
        let load = HexLoadOptions::default();
        let doc = match &format.conversion {
            Some(conversion) => HexDocument::open_converted(
                &*router,
                &spec,
                &ExternalHex {
                    service: self,
                    conversion,
                },
                &load,
                &ctl,
            ),
            None => HexDocument::open(&*router, &spec, &load, &ctl),
        }
        .map_err(|e| match e {
            HexError::Io { source, .. } => ApiError::from_vfs(source),
            other => hex_error(other),
        })?;
        let len = doc.len();
        // An offset past the end is refused (F9: a dump prints what is there; W13's Go To clamps instead); `end` is
        // the last byte; `+n` counts from 0.
        let offset = match offset_query {
            None => 0,
            Some(AddressQuery::End) => len.saturating_sub(1),
            Some(AddressQuery::Absolute(n)) => n,
            Some(AddressQuery::Relative(d)) => u64::try_from(d).unwrap_or(u64::MAX),
        };
        if offset > 0 && offset >= len {
            return Err(ApiError::InvalidArgument(format!(
                "--offset {}: past the end of {} ({len} bytes; the last byte is at {})",
                req.offset.as_deref().unwrap_or_default().trim(),
                spec.canonical(),
                match len {
                    0 => "no offset".to_owned(),
                    n => format!("0x{:X}", n - 1),
                }
            )));
        }
        let take = length.min(len.saturating_sub(offset));
        let mut bytes = vec![0u8; take as usize];
        let mut at = 0usize;
        while at < bytes.len() {
            job.cancel.check().map_err(|_| ApiError::Cancelled)?;
            let n = doc
                .read(offset + at as u64, &mut bytes[at..])
                .map_err(hex_error)?;
            if n == 0 {
                return Err(ApiError::Hex(HexError::Truncated {
                    spec: doc.display().to_owned(),
                }));
            }
            at += n;
        }
        let values = if req.values {
            Some(values::at(&doc, offset).map_err(hex_error)?)
        } else {
            None
        };
        let info = doc.info();
        let side = HexSide {
            spec: spec.canonical(),
            size: len,
            mtime: entry
                .mtime
                .and_then(|t| jiff::Timestamp::new(t.unix_seconds(), t.subsec_nanos() as i32).ok()),
            format: format.format.clone(),
            converted: info.converted,
            load: info.load,
        };
        Ok(HexDumpResponse {
            side,
            layout,
            offset,
            bytes,
            values,
            warnings,
        })
    }

    /// `hex find` (§1.12.23, D-334): the pattern compiled first (a bad pattern is exit 100 before anything is read); the
    /// files of `text find`'s walk ([`FileWalk`]: the same order, masks, hidden items and archives), each opened as a
    /// hex document and searched with [`HexFinder::all`]; the events streamed to `sink` in order; at most
    /// `max_matches` hits (`find_limit_reached`) and `max_matches_per_file` a file.
    pub fn hex_find(
        &self,
        req: &HexFindRequest,
        sink: &mut dyn HexFindSink,
        job: &JobControl,
    ) -> Result<HexFindOutcome, ApiError> {
        run_guarded(|| self.hex_find_inner(req, sink, job))
    }

    fn hex_find_inner(
        &self,
        req: &HexFindRequest,
        sink: &mut dyn HexFindSink,
        job: &JobControl,
    ) -> Result<HexFindOutcome, ApiError> {
        let pattern = hex_pattern_of(req)?;
        let finder = HexFinder::new(&pattern).map_err(vocab)?;
        if req.roots.is_empty() {
            return Err(ApiError::InvalidArgument(
                "no file or folder to search".to_owned(),
            ));
        }
        let positive = |v: Option<u64>, flag: &str, default: u64| match v {
            Some(0) => Err(ApiError::InvalidArgument(format!(
                "{flag} must be at least 1"
            ))),
            Some(n) => Ok(n),
            None => Ok(default),
        };
        let max_matches = positive(req.max_matches, "--max-matches", 100_000)?;
        let per_file = positive(req.max_matches_per_file, "--max-matches-per-file", 10_000)?;
        let mask_options = MaskOptions {
            case: CaseSensitivity::HOST_DEFAULT,
        };
        let masks = match req.mask.as_deref() {
            Some(mask) => NameFilter::parse(mask, mask_options)
                .map_err(|e| ApiError::InvalidArgument(format!("--mask {mask:?}: {e}")))?,
            None => NameFilter::match_all(mask_options),
        };
        let mut roots = Vec::with_capacity(req.roots.len());
        for raw in &req.roots {
            roots.push(self.parse_spec_in(raw, self.use_of(raw), &job.cancel)?);
        }
        let router = self.router();
        let ctl = job.ctl();
        let scope = WalkScope {
            roots: &roots,
            masks: &masks,
            recursive: req.recursive,
            include_hidden: req.include_hidden,
            archives: req.archives,
        };
        let mut walk = FileWalk::new(&*router, scope, &ctl).map_err(|e| match e {
            e if e.is_cancelled() => ApiError::Cancelled,
            hikasa_text::TextError::Load { source, .. } => ApiError::from_vfs(source),
            e => ApiError::Text(e),
        })?;
        let stream = self.stream_options();
        let load = HexLoadOptions::default();
        let mut outcome = HexFindOutcome {
            json: HexFindJson {
                query: query_json(req, &pattern),
                roots: roots.iter().map(Spec::canonical).collect(),
                mask: req.mask.clone(),
                recursive: req.recursive,
                include_hidden: req.include_hidden,
                archives: req.archives.as_str(),
                files: Vec::new(),
                skipped: Vec::new(),
                counts: HexFindCountsJson {
                    files_searched: 0,
                    files_matched: 0,
                    hits: 0,
                    skipped: 0,
                },
                limit_reached: false,
            },
            warnings: Vec::new(),
        };
        while let Some(item) = walk.next_item(&ctl).map_err(|e| {
            if e.is_cancelled() {
                ApiError::Cancelled
            } else {
                ApiError::Text(e)
            }
        })? {
            let (spec, size) = match item {
                WalkItem::Skipped(spec, why) => {
                    let error = match why {
                        hikasa_text::find::files::SkipWhy::Unreadable(e) => Some(e),
                        _ => None,
                    };
                    let skipped = HexSkippedJson {
                        spec: spec.canonical(),
                        reason: if error.is_some() { "error" } else { "filtered" },
                        error,
                    };
                    sink.event(HexFindEvent::Skipped(&skipped));
                    outcome.json.counts.skipped += 1;
                    outcome.json.skipped.push(skipped);
                    continue;
                }
                WalkItem::File(spec, size) => (spec, size),
            };
            let remaining = max_matches - outcome.json.counts.hits;
            let limit = per_file.min(remaining);
            let searched = HexDocument::open(&*router, &spec, &load, &ctl)
                .and_then(|doc| finder.all(&doc, limit, &stream, &ctl));
            let (hits, reached) = match searched {
                Ok(found) => found,
                Err(e) if e.is_cancelled() => return Err(ApiError::Cancelled),
                Err(e) => {
                    let skipped = HexSkippedJson {
                        spec: spec.canonical(),
                        reason: "error",
                        error: Some(ErrorReport::capture(&e, Some(&spec))),
                    };
                    sink.event(HexFindEvent::Skipped(&skipped));
                    outcome.json.counts.skipped += 1;
                    outcome.json.skipped.push(skipped);
                    continue;
                }
            };
            outcome.json.counts.files_searched += 1;
            let canonical = spec.canonical();
            for &at in &hits {
                sink.event(HexFindEvent::Hit {
                    spec: &canonical,
                    offset: at,
                });
            }
            let n = hits.len() as u64;
            outcome.json.counts.hits += n;
            let global_stop = reached && limit < per_file;
            let file = HexFindFileJson {
                spec: canonical,
                size,
                hit_count: n,
                stopped: reached,
                hits: if req.keep_hits { hits } else { Vec::new() },
            };
            if n > 0 {
                outcome.json.counts.files_matched += 1;
                sink.event(HexFindEvent::File(&file));
                outcome.json.files.push(file);
            }
            if outcome.json.counts.hits >= max_matches && (global_stop || reached || n == limit) {
                outcome.json.limit_reached = true;
                outcome.warnings.push(Warning {
                    code: WarningCode::FindLimitReached,
                    message: format!(
                        "the search stopped at --max-matches {max_matches}: the rest was not searched"
                    ),
                    spec: None,
                });
                break;
            }
        }
        Ok(outcome)
    }
}

/// The pattern of a find request.
fn hex_pattern_of(req: &HexFindRequest) -> Result<HexPattern, ApiError> {
    if req.hex {
        if req.encoding.is_some() || req.match_case {
            return Err(ApiError::InvalidArgument(
                "--encoding and --case apply to a text pattern, not to --hex bytes".to_owned(),
            ));
        }
        return Ok(HexPattern::Bytes {
            bytes: hikasa_hex::find::parse_hex_bytes(&req.pattern).map_err(vocab)?,
        });
    }
    let encoding = match &req.encoding {
        Some(w) => TextEncoding::parse(w).map_err(vocab)?,
        None => TextEncoding::Utf8,
    };
    Ok(HexPattern::Text {
        text: req.pattern.clone(),
        encoding,
        match_case: req.match_case,
    })
}

fn query_json(req: &HexFindRequest, pattern: &HexPattern) -> HexQueryJson {
    match pattern {
        HexPattern::Text {
            encoding,
            match_case,
            ..
        } => HexQueryJson {
            kind: "text",
            pattern: req.pattern.clone(),
            encoding: Some(encoding.as_str()),
            match_case: *match_case,
        },
        _ => HexQueryJson {
            kind: "bytes",
            pattern: req.pattern.clone(),
            encoding: None,
            match_case: false,
        },
    }
}

// ---- dump and find types -----------------------------------------------------------------------------------------

/// `hikasa hex dump` (§1.12.23).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HexDumpRequest {
    /// The spec as typed.
    pub spec: OsString,
    /// `--offset <addr>` (decimal digits by default; `0x…`, `…h`, `$…`, `#…`, `end`).
    pub offset: Option<String>,
    /// `--length <n>` (default 256, at most 16 MiB).
    pub length: Option<String>,
    /// The layout words (`--display` and `--context` are not used).
    pub layout: HexLayoutWords,
    /// `--values`.
    pub values: bool,
    /// `--format <id|name>` (a Hex format; its load conversion).
    pub format: Option<String>,
}

impl HexDumpRequest {
    /// Bytes dumped by default.
    pub const DEFAULT_LENGTH: u64 = 256;
    /// The longest dump: 16 MiB.
    pub const MAX_LENGTH: u64 = 16 << 20;
}

/// The result of [`Service::hex_dump`].
#[derive(Debug, Clone)]
pub struct HexDumpResponse {
    /// The side dumped.
    pub side: HexSide,
    /// The layout.
    pub layout: HexLayout,
    /// The first byte dumped.
    pub offset: u64,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// The values at `offset` (`--values`).
    pub values: Option<Values>,
    /// Warnings.
    pub warnings: Vec<Warning>,
}

impl HexDumpResponse {
    /// `hikasa.hex_dump.v1`.
    pub fn json(&self) -> HexDumpJson {
        HexDumpJson {
            side: HexSideJson::new(&self.side),
            layout: HexLayoutJson::new(&self.layout),
            offset: self.offset,
            length: self.bytes.len() as u64,
            rows: hex_dump_rows(&self.layout, self.offset, &self.bytes),
            values: self
                .values
                .as_ref()
                .map(|v| ValuesBlockJson::new(self.offset, v)),
        }
    }

    /// The human output: the rows as the pane shows them (`address  hex  chars`, the layout's groups), then the value
    /// panel with `--values`.
    pub fn human_lines(&self) -> Vec<String> {
        let rows: Vec<HexDumpRowJson> = hex_dump_rows(&self.layout, self.offset, &self.bytes);
        let last = (self.offset + self.bytes.len() as u64).max(self.side.size);
        let width = match self.layout.address {
            Radix::Hex => {
                ((64 - last.saturating_sub(1).leading_zeros()).div_ceil(4) as usize).max(8)
            }
            Radix::Decimal => last
                .saturating_sub(1)
                .checked_ilog10()
                .map_or(1, |d| d as usize + 1),
        };
        let bpl = usize::from(self.layout.bytes_per_line.clamp(1, 256));
        let group = usize::from(self.layout.group.max(1));
        let mut out = Vec::with_capacity(rows.len() + 24);
        for (i, row) in rows.iter().enumerate() {
            let chunk = &self.bytes[i * bpl..((i + 1) * bpl).min(self.bytes.len())];
            let mut hex = String::new();
            for k in 0..bpl {
                if k > 0 {
                    hex.push_str(if group > 1 && k % group == 0 {
                        "  "
                    } else {
                        " "
                    });
                }
                match chunk.get(k) {
                    Some(b) => hex.push_str(&format!("{b:02x}")),
                    None => hex.push_str("  "),
                }
            }
            let addr = match self.layout.address {
                Radix::Hex => format!("{:0width$X}", row.address),
                Radix::Decimal => format!("{:>width$}", row.address),
            };
            let line = match &row.chars {
                Some(chars) => format!("{addr}  {hex}  {chars}"),
                None => format!("{addr}  {hex}"),
            };
            out.push(line.trim_end().to_owned());
        }
        if let Some(v) = &self.values {
            out.push(String::new());
            out.push(format!(
                "values at {}:",
                nav::address_text(Side::Left, self.offset, Radix::Hex, 8)
            ));
            out.extend(values::format(v).lines().map(str::to_owned));
        }
        out
    }
}

/// `hikasa hex find` (§1.12.23).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HexFindRequest {
    /// The pattern as typed: text, or hex bytes with `hex`.
    pub pattern: String,
    /// Files and folders searched, in this order.
    pub roots: Vec<OsString>,
    /// `--hex`: the pattern is hex bytes (`DE AD ?? EF`).
    pub hex: bool,
    /// `--encoding utf-8|latin1|utf-16le|utf-16be` (text).
    pub encoding: Option<String>,
    /// `--case`: ASCII letters match exactly (text).
    pub match_case: bool,
    /// `--recursive`.
    pub recursive: bool,
    /// `--mask <masks>`.
    pub mask: Option<String>,
    /// `--include-hidden`.
    pub include_hidden: bool,
    /// `--archives files|opened`.
    pub archives: FindArchives,
    /// `--max-matches <n>` (default 100,000).
    pub max_matches: Option<u64>,
    /// `--max-matches-per-file <n>` (default 10,000).
    pub max_matches_per_file: Option<u64>,
    /// The JSON keeps every hit's offset (`--json` without `--count`/`--files-with-matches`).
    pub keep_hits: bool,
}

/// One event of a hex find, in walk order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexFindEvent<'a> {
    /// A hit.
    Hit {
        /// The file.
        spec: &'a str,
        /// The hit's offset.
        offset: u64,
    },
    /// A file with hits, after its hits.
    File(&'a HexFindFileJson),
    /// A file or folder not searched.
    Skipped(&'a HexSkippedJson),
}

/// Receives the events of [`Service::hex_find`].
pub trait HexFindSink {
    /// One event.
    fn event(&mut self, e: HexFindEvent<'_>);
}

/// A sink that drops every event.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHexEvents;

impl HexFindSink for NoHexEvents {
    fn event(&mut self, _: HexFindEvent<'_>) {}
}

/// The result of [`Service::hex_find`].
#[derive(Debug, Clone)]
pub struct HexFindOutcome {
    /// `hikasa.hex_find.v1`.
    pub json: HexFindJson,
    /// `find_limit_reached`.
    pub warnings: Vec<Warning>,
}

// ---- sessions (D-349) --------------------------------------------------------------------------------------------

/// `[rules.hex]` of a `hex-compare` session (D-349): the rule words with sizes as numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexRulesFile {
    /// `none`, `fast`, `complete`.
    pub alignment: String,
    /// Fast's average block in bytes.
    pub block: u64,
    /// The skew limit in bytes.
    pub skew: u64,
    /// Complete's window in bytes.
    pub window: u64,
    /// Complete's budget in MiB.
    pub budget_mib: u64,
    /// HX-002's unlocked maximum in MiB.
    pub unlocked_max_mib: u64,
}

impl Default for HexRulesFile {
    fn default() -> HexRulesFile {
        HexRulesFile {
            alignment: AlignMode::Fast.as_str().to_owned(),
            block: u64::from(HexAlignOptions::DEFAULT_BLOCK),
            skew: HexAlignOptions::DEFAULT_SKEW,
            window: u64::from(HexAlignOptions::DEFAULT_WINDOW),
            budget_mib: HexAlignOptions::DEFAULT_BUDGET_MIB,
            unlocked_max_mib: HexLoadOptions::DEFAULT_UNLOCKED_MAX >> 20,
        }
    }
}

impl HexRulesFile {
    /// The rule words of the file's values.
    pub fn words(&self) -> HexRuleWords {
        HexRuleWords {
            alignment: Some(self.alignment.clone()),
            block: Some(self.block.to_string()),
            skew: Some(self.skew.to_string()),
            window: Some(self.window.to_string()),
            budget_mib: Some(self.budget_mib.to_string()),
            unlocked_max_mib: Some(self.unlocked_max_mib.to_string()),
        }
    }

    /// The file's values of rule words (absent words: the defaults).
    pub fn of_words(words: &HexRuleWords) -> Result<HexRulesFile, ApiError> {
        let (align, load) = hex_rules_of(words)?;
        Ok(HexRulesFile {
            alignment: align.mode.as_str().to_owned(),
            block: u64::from(align.block),
            skew: align.skew,
            window: u64::from(align.window),
            budget_mib: align.budget_mib,
            unlocked_max_mib: load.unlocked_max >> 20,
        })
    }
}

/// The rules of a `hex-compare` session (`[rules]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexSessionRules {
    /// `[rules.hex]`.
    pub hex: HexRulesFile,
}

/// A string or a number of a `[view]` key as a word.
pub(crate) fn view_word(table: &toml::Table, key: &str) -> Option<String> {
    match table.get(key)? {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(n) => Some(n.to_string()),
        toml::Value::Boolean(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

/// The layout words of a session's `[view]` and `[display]` (D-349).
pub fn hex_layout_words_of(view: &toml::Table, display: &toml::Table) -> HexLayoutWords {
    HexLayoutWords {
        bytes_per_line: view_word(view, "bytes_per_line"),
        group: view_word(view, "group"),
        chars: view_word(view, "chars"),
        address: view_word(view, "address"),
        context: view_word(view, "context"),
        display: view_word(display, "filter"),
    }
}

/// Checks a hex session's words through the parsers (`session edit --set`, D-349): the rules, the `[view]` layout keys
/// and `[display] filter`; `[view] layout` is `side-by-side` or `over-under`.
pub(crate) fn check_hex_session(
    rules: &HexSessionRules,
    view: &toml::Table,
    display: &toml::Table,
) -> Result<(), ApiError> {
    hex_rules_of(&rules.hex.words())?;
    hex_layout_of(&hex_layout_words_of(view, display), None)?;
    if let Some(layout) = view_word(view, "layout")
        && !matches!(layout.as_str(), "side-by-side" | "over-under")
    {
        return Err(ApiError::InvalidArgument(format!(
            "invalid view.layout {layout:?}: expected side-by-side, over-under"
        )));
    }
    Ok(())
}

impl Service {
    /// The rule words `hex compare` runs with (D-374, W12c review fixes F10): the words typed on top of the Hex Compare
    /// defaults session (`[rules.hex]`), as `session new --type hex-compare` starts from it. A word typed wins; an
    /// absent one is the session's. The session's `[view]` and `[display]` stay a view's: the command's layout is its
    /// flags. A defaults session that cannot be read leaves the words as typed.
    pub fn hex_rules_on_defaults(&self, typed: &HexRuleWords) -> HexRuleWords {
        let defaults = match self.session_defaults(crate::sessions::SessionType::HexCompare) {
            Ok(session) => match &session.rules {
                crate::sessions::SessionRules::Hex(rules) => rules.hex.words(),
                _ => return typed.clone(),
            },
            Err(_) => return typed.clone(),
        };
        HexRuleWords {
            alignment: typed.alignment.clone().or(defaults.alignment),
            block: typed.block.clone().or(defaults.block),
            skew: typed.skew.clone().or(defaults.skew),
            window: typed.window.clone().or(defaults.window),
            budget_mib: typed.budget_mib.clone().or(defaults.budget_mib),
            unlocked_max_mib: typed.unlocked_max_mib.clone().or(defaults.unlocked_max_mib),
        }
    }
}
