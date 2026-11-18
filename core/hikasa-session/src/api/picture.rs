//! Picture Compare in the Service (§1.12.22, D-345; W12c): [`PictureCompareRequest`] with the user's words
//! ([`PictureRuleWords`], [`PictureGeometryWords`]), [`Service::picture_load`], [`Service::picture_diff`],
//! [`Service::picture_compare`] (load, diff, the verdict of D-339, probes, `--diff-output`), [`Service::picture_write`]
//! (a pane as a PNG to a file, stdout or the clipboard: [`ClipboardImage`] for the adapter), the theme's colours
//! ([`Service::picture_colors`]), the human output ([`PictureCompareResponse::human_lines`]) and the folder content
//! test's comparator (D-344, [`Service::picture_comparator`]).
//!
//! Every word of pictures is parsed here, by the parsers of `hikasa-picture` (check 7).

use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::{ContentKind, ExitCode, Pair, Side, Spec, Warning};
use hikasa_formats::{Conversion, ExternalConversion, FormatKind, run_external};
use hikasa_picture::vocab as words;
use hikasa_picture::{
    DecodeOptions, DiffMode, Encoded, Geometry, Orientation, Pane, Picture, PictureColors,
    PictureComparator, PictureComparison, PictureConverter, PictureError, PictureOptions,
    PictureWarning, PixelProbe, PngLevel, ToneMap, compare_pictures, decode_two, decode_vec,
    encode_png, read_encoded,
};
use hikasa_report::json::{FormatJson, PictureCompareJson};
use hikasa_report::{
    DiffOutput, PictureSide, PictureSource, picture_compare_json, probe_line, side_line, stats_line,
};
use hikasa_vfs::{EntryKind, Vfs, VfsSource, compare_sources};
use serde::{Deserialize, Serialize};

use super::compare::load_failed;
use super::report::ReportTarget;
use super::table::format_json;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

fn is_none<T>(v: &Option<T>) -> bool {
    v.is_none()
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The rule words of a picture comparison as typed (§1.12.23; a session's `[rules.picture]`, D-349).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PictureRuleWords {
    /// `--mode tolerance|range|blend`.
    #[serde(skip_serializing_if = "is_none")]
    pub mode: Option<String>,
    /// `--tolerance <0-255>`.
    #[serde(skip_serializing_if = "is_none")]
    pub tolerance: Option<String>,
    /// `--blend <0-100>` (blend mode only).
    #[serde(skip_serializing_if = "is_none")]
    pub blend: Option<String>,
    /// `--auto-scale`.
    #[serde(skip_serializing_if = "is_false")]
    pub auto_scale: bool,
    /// `--tone-map clamp|reinhard`.
    #[serde(skip_serializing_if = "is_none")]
    pub tone_map: Option<String>,
    /// `--max-megapixels <n>`.
    #[serde(skip_serializing_if = "is_none")]
    pub max_megapixels: Option<String>,
    /// `--ignore-unimportant`.
    #[serde(skip_serializing_if = "is_false")]
    pub ignore_unimportant: bool,
}

/// The geometry words as typed (§1.12.17; a session's `[view]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PictureGeometryWords {
    /// `--left-orientation <list>`.
    #[serde(skip_serializing_if = "is_none")]
    pub left_orientation: Option<String>,
    /// `--right-orientation <list>`.
    #[serde(skip_serializing_if = "is_none")]
    pub right_orientation: Option<String>,
    /// `--offset <dx>,<dy>`.
    #[serde(skip_serializing_if = "is_none")]
    pub offset: Option<String>,
}

/// The words of a Picture Compare view in an `OpenView` (D-350).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PictureWords {
    /// The rule words.
    pub rules: PictureRuleWords,
    /// The geometry words.
    pub geometry: PictureGeometryWords,
    /// `--left-format`, `--right-format`.
    pub formats: [Option<String>; 2],
}

/// `hikasa picture compare` (§1.12.23).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PictureCompareRequest {
    /// Left spec as typed.
    pub left: OsString,
    /// Right spec as typed.
    pub right: OsString,
    /// `--format <id|name>`.
    pub format: Option<String>,
    /// `--left-format`, `--right-format`.
    pub formats: Pair<Option<String>>,
    /// The rule words.
    pub rules: PictureRuleWords,
    /// The geometry words.
    pub geometry: PictureGeometryWords,
    /// `--probe <x>,<y>` (repeatable).
    pub probes: Vec<String>,
    /// `--diff-output <spec>` (`-`: stdout; `clipboard://`).
    pub diff_output: Option<OsString>,
    /// `--diff-pane difference|left|right`.
    pub diff_pane: Option<String>,
    /// `--checkerboard`.
    pub checkerboard: bool,
    /// A session's `[view] checkerboard` (D-349; default on): the `--diff-output` image's checkerboard when
    /// `--checkerboard` is not given (F11). `None` outside a session.
    pub view_checkerboard: Option<bool>,
    /// `--metadata` (refused: PX-006 is P1).
    pub metadata: bool,
    /// FF-002.
    pub format_overrides: std::collections::BTreeMap<String, bool>,
    /// `--verbose`: the informational warnings too (`frames_ignored`, `orientation_applied`).
    pub verbose: bool,
    /// `--dry-run` (F4): the `--diff-output` image is planned ([`PictureCompareResponse::planned`]), never written.
    pub dry_run: bool,
}

impl PictureCompareRequest {
    /// A picture comparison of two specs that were already parsed.
    pub fn for_specs(left: &Spec, right: &Spec) -> PictureCompareRequest {
        PictureCompareRequest {
            left: super::spec_os_string(left),
            right: super::spec_os_string(right),
            ..PictureCompareRequest::default()
        }
    }

    /// The words of a view (D-350).
    pub fn words(&self) -> PictureWords {
        PictureWords {
            rules: self.rules.clone(),
            geometry: self.geometry.clone(),
            formats: [self.formats.left.clone(), self.formats.right.clone()],
        }
    }
}

/// The parsed settings of a picture request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureSettings {
    /// Decoding.
    pub decode: DecodeOptions,
    /// Tolerance and geometry.
    pub options: PictureOptions,
    /// The display mode.
    pub mode: DiffMode,
    /// `--ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// The probes' positions.
    pub probes: Vec<(u32, u32)>,
    /// The pane `--diff-output` writes.
    pub pane: Pane,
}

fn vocab(e: words::VocabError) -> ApiError {
    ApiError::InvalidArgument(e.to_string())
}

/// Parses the rule and geometry words.
pub fn picture_settings_of(
    rules: &PictureRuleWords,
    geometry: &PictureGeometryWords,
) -> Result<PictureSettings, ApiError> {
    let mut mode = match &rules.mode {
        Some(w) => DiffMode::parse(w).map_err(vocab)?,
        None => DiffMode::Tolerance,
    };
    if let Some(w) = &rules.blend {
        let percent = words::parse_blend(w).map_err(vocab)?;
        match &mut mode {
            DiffMode::Blend { percent: p } => *p = percent,
            _ => {
                return Err(ApiError::InvalidArgument(
                    "--blend applies to --mode blend".to_owned(),
                ));
            }
        }
    }
    let tolerance = match &rules.tolerance {
        Some(w) => words::parse_tolerance(w).map_err(vocab)?,
        None => 0,
    };
    let mut decode = DecodeOptions::default();
    if let Some(w) = &rules.tone_map {
        decode.tone_map = ToneMap::parse(w).map_err(vocab)?;
    }
    if let Some(w) = &rules.max_megapixels {
        decode.max_pixels = words::parse_max_megapixels(w).map_err(vocab)?;
    }
    let orientation = |w: &Option<String>| match w {
        Some(w) => Orientation::parse_list(w).map_err(vocab),
        None => Ok(Orientation::Identity),
    };
    let geometry = Geometry {
        orientation: Pair::new(
            orientation(&geometry.left_orientation)?,
            orientation(&geometry.right_orientation)?,
        ),
        auto_scale: rules.auto_scale,
        offset: match &geometry.offset {
            Some(w) => words::parse_offset(w).map_err(vocab)?,
            None => (0, 0),
        },
    };
    geometry.check().map_err(vocab)?;
    Ok(PictureSettings {
        decode,
        options: PictureOptions {
            tolerance,
            geometry,
        },
        mode,
        ignore_unimportant: rules.ignore_unimportant,
        probes: Vec::new(),
        pane: Pane::Difference,
    })
}

/// The format a side was decoded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureFormatChoice {
    /// The Hikasa format of the Picture kind (`None`: none matched; the container decides).
    pub format: Option<FormatJson>,
    /// Its load conversion.
    pub conversion: Option<ExternalConversion>,
}

/// Both pictures, decoded ([`Service::picture_load`]).
#[derive(Debug, Clone)]
pub struct PictureLoaded {
    /// The specs.
    pub specs: Pair<Spec>,
    /// The pictures.
    pub pictures: Pair<Arc<Picture>>,
    /// The formats.
    pub formats: Pair<PictureFormatChoice>,
    /// What the reports and the JSON show of each side.
    pub sides: Pair<PictureSide>,
    /// The tone map they were decoded with.
    pub tone_map: ToneMap,
}

/// An image the adapter places on the clipboard (`ClipboardProvider::write_image`, D-343; check 2: the Service
/// never touches the clipboard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8, row-major.
    pub rgba: Vec<u8>,
}

/// What [`Service::picture_write`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureWritten {
    /// Width of the image.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// The file written (a file target).
    pub spec: Option<Spec>,
    /// The PNG bytes the adapter writes to its stdout (the `-` target).
    pub stdout: Option<Vec<u8>>,
    /// The image the adapter places on the clipboard (the `clipboard://` target).
    pub clipboard: Option<ClipboardImage>,
}

/// What a dry run would do with the `--diff-output` image (F4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedImage {
    /// `write <spec>`, `replace <spec>`, `write the image to standard output` or `copy the image to the clipboard`.
    pub line: String,
    /// A file would be written (counted by `would write <n> files`).
    pub file: bool,
}

/// The target of `--diff-output` (F7): `-` is standard output, `clipboard://` the clipboard, any other word a file
/// (`clipboard`, `printer` and `browser` included: unlike `--output-to`, the flag names an image file).
pub fn diff_output_target(raw: &std::ffi::OsStr) -> ReportTarget {
    match raw.to_str() {
        Some("-") => ReportTarget::Stdout,
        Some(t) if t.eq_ignore_ascii_case("clipboard://") => ReportTarget::Clipboard,
        _ => ReportTarget::File(raw.to_owned()),
    }
}

/// The result of [`Service::picture_compare`].
#[derive(Debug, Clone)]
pub struct PictureCompareResponse {
    /// Both pictures.
    pub loaded: PictureLoaded,
    /// The comparison.
    pub comparison: Arc<PictureComparison>,
    /// The verdict (D-339).
    pub kind: ContentKind,
    /// The exit code (1, 2, 12, 13).
    pub exit_code: ExitCode,
    /// The display mode.
    pub mode: DiffMode,
    /// `--ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// The probes.
    pub probes: Vec<PixelProbe>,
    /// The `--diff-output` image.
    pub diff_output: Option<DiffOutput>,
    /// What the adapter still writes of it (stdout bytes, a clipboard image).
    pub written: Option<PictureWritten>,
    /// A dry run's plan of the image (F4): the line `--dry-run` lists, and whether it is a file it would write.
    pub planned: Option<PlannedImage>,
    /// The theme's colours.
    pub colors: PictureColors,
    /// Warnings.
    pub warnings: Vec<Warning>,
}

impl PictureCompareResponse {
    /// The input of the reports and of the JSON document.
    pub fn source(&self) -> PictureSource<'_> {
        PictureSource {
            comparison: &self.comparison,
            sides: Pair::new(&self.loaded.sides.left, &self.loaded.sides.right),
            kind: self.kind,
            ignore_unimportant: self.ignore_unimportant,
            mode: self.mode,
            tone_map: self.loaded.tone_map,
            colors: &self.colors,
            probes: &self.probes,
            diff_output: self.diff_output.as_ref(),
            warnings: &self.warnings,
        }
    }

    /// `hikasa.picture_compare.v1`.
    pub fn json(&self) -> PictureCompareJson {
        picture_compare_json(&self.source())
    }

    /// The human output (§1.12.23): a line per side, the statistics line, each probe. The caller prints the verdict
    /// line.
    pub fn human_lines(&self) -> Vec<String> {
        let pictures = self.comparison.pictures();
        let mut out = vec![
            side_line(
                &self.loaded.sides.left.spec,
                &pictures.left.info,
                Side::Left,
            ),
            side_line(
                &self.loaded.sides.right.spec,
                &pictures.right.info,
                Side::Right,
            ),
            stats_line(&self.comparison),
        ];
        out.extend(self.probes.iter().map(probe_line));
        out
    }
}

/// What a view adds to its comparison for [`Service::picture_response`] (M2 W13b): the display mode, Ignore
/// Unimportant, whether the inputs are byte-identical (asked once, D-339), the colours and `verbose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureViewState {
    /// The display mode.
    pub mode: DiffMode,
    /// Ignore Unimportant.
    pub ignore_unimportant: bool,
    /// The inputs hold the same bytes (`false` while unknown).
    pub bytes_identical: bool,
    /// The colours of the view.
    pub colors: PictureColors,
    /// Informational decoder warnings too.
    pub verbose: bool,
}

/// A picture warning as a [`Warning`].
pub fn picture_warning(w: &PictureWarning) -> Warning {
    w.to_warning()
}

fn of<T>(pair: &Pair<T>, side: Side) -> &T {
    match side {
        Side::Left => &pair.left,
        _ => &pair.right,
    }
}

/// A picture error on `side` as an API error: a read failure is `LoadFailed`; a cancellation `Cancelled`; any other
/// error names the spec.
fn picture_error(e: PictureError, side: Side, spec: &Spec) -> ApiError {
    match e {
        PictureError::Cancelled => ApiError::Cancelled,
        PictureError::Io { source, .. } => load_failed(side, spec, source),
        other => ApiError::Picture(other, Some(spec.canonical())),
    }
}

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

/// The largest input a picture load conversion reads (2 GiB, the decoder's own limit).
const CONVERT_MAX: u64 = 2 << 30;

/// Runs a Picture format's External load conversion of `input` (the content of `display`): the one conversion of
/// `picture compare`, the folder content test and `quick --rules` (T11, F1).
fn convert_picture(
    conversion: &ExternalConversion,
    input: &[u8],
    display: &str,
    temp: &std::path::Path,
    runner: &dyn hikasa_formats::ProcessRunner,
    ctl: &hikasa_core::Ctl<'_>,
) -> Result<Vec<u8>, PictureError> {
    run_external(conversion, input, display, temp, runner, ctl.cancel).map_err(|e| match e {
        hikasa_formats::ConvertError::Cancelled => PictureError::Cancelled,
        other => PictureError::Decode {
            format: "conversion".to_owned(),
            message: other.to_string(),
        },
    })
}

/// The Service's [`PictureConverter`] (F1): a name's Picture format by the registry (or the forced one), its External
/// conversion through the Service's runner in its temp folder.
struct PictureConversions {
    registry: Arc<hikasa_formats::FormatRegistry>,
    overrides: hikasa_formats::EnabledOverrides,
    forced: Option<ExternalConversion>,
    temp: std::path::PathBuf,
    runner: Arc<dyn hikasa_formats::ProcessRunner>,
}

impl std::fmt::Debug for PictureConversions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureConversions")
            .field("forced", &self.forced)
            .finish_non_exhaustive()
    }
}

impl PictureConverter for PictureConversions {
    fn convert(
        &self,
        name: &str,
        input: &[u8],
        ctl: &hikasa_core::Ctl<'_>,
    ) -> Result<Option<Vec<u8>>, PictureError> {
        let conversion = match &self.forced {
            Some(forced) => Some(forced.clone()),
            None => self
                .registry
                .detect(name, &self.overrides)
                .ok()
                .filter(|d| d.format.kind() == FormatKind::Picture)
                .and_then(|d| match d.format.conversion() {
                    Conversion::External(x) => Some(x.clone()),
                    _ => None,
                }),
        };
        match conversion {
            Some(c) => convert_picture(&c, input, name, &self.temp, &*self.runner, ctl).map(Some),
            None => Ok(None),
        }
    }
}

impl Service {
    /// The words of a picture request checked as [`Service::picture_compare`] checks them before anything is read: the
    /// refusals of §1.12.23, the rule and geometry words, the probes, the pane and the forced formats (known, of the
    /// Picture kind). What `picture compare --gui` checks locally (D-350).
    pub fn check_picture_words(
        &self,
        req: &PictureCompareRequest,
    ) -> Result<PictureSettings, ApiError> {
        run_guarded(|| {
            if req.metadata {
                return Err(ApiError::Unsupported(
                    "picture metadata comparison is not available in this version".to_owned(),
                ));
            }
            if req.diff_output.is_none() && (req.checkerboard || req.diff_pane.is_some()) {
                return Err(ApiError::InvalidArgument(
                    "--checkerboard and --diff-pane apply to --diff-output".to_owned(),
                ));
            }
            let mut settings = picture_settings_of(&req.rules, &req.geometry)?;
            for p in &req.probes {
                settings.probes.push(words::parse_point(p).map_err(vocab)?);
            }
            if let Some(w) = &req.diff_pane {
                settings.pane = words::Pane::parse(w).map_err(vocab)?;
            }
            format_combination(&req.format, &req.formats)?;
            for word in [&req.format, &req.formats.left, &req.formats.right]
                .into_iter()
                .flatten()
            {
                self.picture_format_forced(word)?;
            }
            Ok(settings)
        })
    }

    /// A forced format: known and of the Picture kind.
    fn picture_format_forced(&self, word: &str) -> Result<hikasa_formats::Detection, ApiError> {
        let detection = self.registry().force(word)?;
        if detection.format.kind() != FormatKind::Picture {
            return Err(ApiError::InvalidArgument(format!(
                "format `{}` is a {} format: picture compare takes a format of the picture kind",
                detection.format.name(),
                detection.format.kind()
            )));
        }
        Ok(detection)
    }

    fn picture_format(
        &self,
        spec: &Spec,
        own: Option<&str>,
        both: Option<&str>,
        overrides: &hikasa_formats::EnabledOverrides,
    ) -> Result<PictureFormatChoice, ApiError> {
        let detection = match own.or(both) {
            Some(word) => Some(self.picture_format_forced(word)?),
            None => self
                .detection_name(spec)
                .and_then(|name| self.registry().detect(&name, overrides).ok())
                .filter(|d| d.format.kind() == FormatKind::Picture),
        };
        Ok(match detection {
            Some(d) => PictureFormatChoice {
                format: Some(format_json(&d)),
                conversion: match d.format.conversion() {
                    Conversion::External(x) => Some(x.clone()),
                    _ => None,
                },
            },
            None => PictureFormatChoice {
                format: None,
                conversion: None,
            },
        })
    }

    /// One side ready to decode: through its format's load conversion (D-336) when it has one (the program's output,
    /// and the input's size), else the file itself.
    fn encoded_side(
        &self,
        spec: &Spec,
        format: &PictureFormatChoice,
        opts: &DecodeOptions,
        ctl: &hikasa_core::Ctl<'_>,
    ) -> Result<(Encoded, Option<u64>), PictureError> {
        let router = self.router();
        let Some(conversion) = &format.conversion else {
            return Ok((read_encoded(&*router, spec, opts, ctl)?, None));
        };
        let display = spec.canonical();
        let mut reader = router
            .open_read_with(spec, ctl.cancel)
            .map_err(|e| PictureError::from_vfs(&display, e))?;
        let mut input = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(&mut reader, CONVERT_MAX + 1),
            &mut input,
        )
        .map_err(|e| {
            PictureError::from_vfs(
                &display,
                hikasa_vfs::VfsError::from_io(display.clone(), hikasa_vfs::VfsOp::Read, e),
            )
        })?;
        if input.len() as u64 > CONVERT_MAX {
            return Err(PictureError::from_vfs(
                &display,
                hikasa_vfs::VfsError::TooLarge {
                    spec: display.clone(),
                    limit: CONVERT_MAX,
                },
            ));
        }
        let (temp, runner) = self.external();
        let output = convert_picture(conversion, &input, &display, temp, runner, ctl)?;
        Ok((
            Encoded {
                bytes: output,
                name: None,
            },
            Some(input.len() as u64),
        ))
    }

    /// The load conversions of the folder content test and `quick --rules` (F1): each name's Picture format (with
    /// `overrides`), or `forced` for both sides; the same conversion as [`Service::picture_load`]'s.
    pub(crate) fn picture_converter(
        &self,
        overrides: hikasa_formats::EnabledOverrides,
        forced: Option<ExternalConversion>,
    ) -> Arc<dyn PictureConverter> {
        Arc::new(PictureConversions {
            registry: self.registry(),
            overrides,
            forced,
            temp: self.env().temp_dir.clone(),
            runner: Arc::clone(self.runner()),
        })
    }

    /// Decodes both pictures (§1.12.22): the words parsed first; each side's Picture format; both decoded in parallel
    /// on the pool (RPT-004's per-side errors: the left one first).
    pub fn picture_load(
        &self,
        req: &PictureCompareRequest,
        job: &JobControl,
    ) -> Result<PictureLoaded, ApiError> {
        run_guarded(|| {
            let settings = self.check_picture_words(req)?;
            self.picture_load_with(req, &settings.decode, job)
        })
    }

    fn picture_load_with(
        &self,
        req: &PictureCompareRequest,
        opts: &DecodeOptions,
        job: &JobControl,
    ) -> Result<PictureLoaded, ApiError> {
        let left = self.parse_input(None, &req.left, &job.cancel)?;
        let right = self.parse_input(None, &req.right, &job.cancel)?;
        let specs = Pair::new(left, right);
        let mut entries = Vec::with_capacity(2);
        for side in [Side::Left, Side::Right] {
            let spec = of(&specs, side);
            let entry = self.stat_input(side, spec, job)?;
            if entry.kind == EntryKind::Dir {
                return Err(ApiError::InvalidArgument(format!(
                    "{}: picture compare compares files; use `folder compare` for folders",
                    spec.canonical()
                )));
            }
            entries.push(entry);
        }
        let mut warnings = Vec::new();
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let formats = Pair::new(
            self.picture_format(
                &specs.left,
                req.formats.left.as_deref(),
                req.format.as_deref(),
                &overrides,
            )?,
            self.picture_format(
                &specs.right,
                req.formats.right.as_deref(),
                req.format.as_deref(),
                &overrides,
            )?,
        );
        let ctl = job.ctl();
        let (left, right) = self.pool().install(|| {
            // The left side is read (and converted) first. When it alone takes half the parallel budget it is
            // decoded before the right file is read, so a large pair never holds both files and a decoder's buffer
            // at once; else the right side is read too and `decode_two` decodes both, in parallel when they fit
            // the budget (F3).
            let one = |e: Encoded| decode_vec(e.bytes, e.name.as_deref(), opts, &ctl);
            let right_side = || self.encoded_side(&specs.right, &formats.right, opts, &ctl);
            let (l, r) = match self.encoded_side(&specs.left, &formats.left, opts, &ctl) {
                Ok((l, lc))
                    if hikasa_picture::decode_peak(&l.bytes, l.name.as_deref(), opts)
                        > hikasa_picture::PARALLEL_DECODE_BUDGET / 2 =>
                {
                    let l = one(l).map(|p| (p, lc));
                    let r = right_side().and_then(|(r, rc)| Ok((one(r)?, rc)));
                    (l, r)
                }
                Ok((l, lc)) => match right_side() {
                    Ok((r, rc)) => {
                        let pair = decode_two(l, r, opts, &ctl);
                        (pair.left.map(|p| (p, lc)), pair.right.map(|p| (p, rc)))
                    }
                    Err(e) => (one(l).map(|p| (p, lc)), Err(e)),
                },
                Err(e) => (Err(e), right_side().and_then(|(r, rc)| Ok((one(r)?, rc)))),
            };
            let converted = |(mut picture, input): (Picture, Option<u64>)| {
                if let Some(size) = input {
                    picture.info.converted = true;
                    picture.info.file_size = size;
                }
                picture
            };
            (l.map(converted), r.map(converted))
        });
        let left = left.map_err(|e| picture_error(e, Side::Left, &specs.left))?;
        let right = right.map_err(|e| picture_error(e, Side::Right, &specs.right))?;
        let pictures = Pair::new(Arc::new(left), Arc::new(right));
        let side_of = |side: Side, entry: &hikasa_vfs::Entry| PictureSide {
            spec: of(&specs, side).canonical(),
            size: entry.size,
            mtime: entry
                .mtime
                .and_then(|t| jiff::Timestamp::new(t.unix_seconds(), t.subsec_nanos() as i32).ok()),
            format: of(&formats, side).format.clone(),
        };
        let sides = Pair::new(
            side_of(Side::Left, &entries[0]),
            side_of(Side::Right, &entries[1]),
        );
        Ok(PictureLoaded {
            specs,
            pictures,
            formats,
            sides,
            tone_map: opts.tone_map,
        })
    }

    /// Compares two decoded pictures (§1.12.22) on the pool.
    pub fn picture_diff(
        &self,
        loaded: &PictureLoaded,
        options: &PictureOptions,
        job: &JobControl,
    ) -> Result<Arc<PictureComparison>, ApiError> {
        run_guarded(|| {
            let ctl = job.ctl();
            let compared = self.pool().install(|| {
                compare_pictures(
                    Arc::clone(&loaded.pictures.left),
                    Arc::clone(&loaded.pictures.right),
                    options,
                    &ctl,
                )
            });
            compared
                .map(Arc::new)
                .map_err(|e| picture_error(e, Side::Left, &loaded.specs.left))
        })
    }

    /// The theme's `[picture]` colours (UI-023) of the selected scheme; `follow_os` resolves to the light theme
    /// headless, as reports do (§1.12.22).
    pub fn picture_colors(&self) -> PictureColors {
        let Ok(snapshot) = self.settings_snapshot() else {
            return PictureColors::default();
        };
        let mode = snapshot.options.value().appearance.theme_mode;
        snapshot.scheme(mode, false).picture.to_render()
    }

    /// Writes a pane of a comparison as a PNG (§1.12.22, PX-009): rendered at full resolution in `mode` with the
    /// theme's colours (and the checkerboard), encoded with the fast PNG level; a file target is written through the
    /// VFS and replaced atomically (an archive member included, D-149); stdout and the clipboard are the adapter's:
    /// [`PictureWritten::stdout`], [`PictureWritten::clipboard`]. The printer and the browser are refused.
    pub fn picture_write(
        &self,
        comparison: &PictureComparison,
        pane: Pane,
        mode: DiffMode,
        checkerboard: bool,
        target: &ReportTarget,
        job: &JobControl,
    ) -> Result<PictureWritten, ApiError> {
        let colors = self.picture_colors();
        self.picture_write_with(comparison, pane, mode, checkerboard, &colors, target, job)
    }

    /// [`Service::picture_write`] with the colours given (M2 W13b, D-365): a window's Copy renders what it shows,
    /// in the window's resolved scheme (`PictureColours::to_render`), not the headless light one. Additive.
    ///
    /// # Errors
    ///
    /// As [`Service::picture_write`].
    #[allow(clippy::too_many_arguments)]
    pub fn picture_write_with(
        &self,
        comparison: &PictureComparison,
        pane: Pane,
        mode: DiffMode,
        checkerboard: bool,
        colors: &PictureColors,
        target: &ReportTarget,
        job: &JobControl,
    ) -> Result<PictureWritten, ApiError> {
        run_guarded(|| {
            if matches!(target, ReportTarget::Printer | ReportTarget::Browser) {
                return Err(ApiError::InvalidArgument(
                    "an image is written to a file, `-` or clipboard://".to_owned(),
                ));
            }
            let image = self
                .pool()
                .install(|| comparison.render(pane, mode, colors, checkerboard, &job.ctl()))
                .map_err(|e| match e {
                    PictureError::Cancelled => ApiError::Cancelled,
                    other => ApiError::Picture(other, None),
                })?;
            let mut written = PictureWritten {
                width: image.width,
                height: image.height,
                spec: None,
                stdout: None,
                clipboard: None,
            };
            if let ReportTarget::Clipboard = target {
                written.clipboard = Some(ClipboardImage {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                });
                return Ok(written);
            }
            match target {
                ReportTarget::Stdout => {
                    written.stdout = Some(
                        encode_png(&image, PngLevel::Fast)
                            .map_err(|e| ApiError::Picture(e, None))?,
                    );
                }
                ReportTarget::File(raw) => {
                    // Compressed straight into the file: the PNG is never held whole (F3).
                    let (spec, _) = self.write_output_with(raw, job, &mut |w| {
                        hikasa_picture::encode_png_to(&image, PngLevel::Fast, w)
                            .map_err(|e| ApiError::Report(hikasa_report::ReportError::Io(e)))
                    })?;
                    written.spec = Some(spec);
                }
                _ => {}
            }
            Ok(written)
        })
    }

    /// Picture compare (§1.12.22): the words checked (the refusals of §1.12.23 first); both pictures decoded;
    /// compared; the verdict of D-339 (the bytes asked only when every position is same); the probes; the
    /// `--diff-output` image. Informational warnings only with `verbose`.
    pub fn picture_compare(
        &self,
        req: &PictureCompareRequest,
        job: &JobControl,
    ) -> Result<PictureCompareResponse, ApiError> {
        run_guarded(|| self.picture_compare_inner(req, job))
    }

    fn picture_compare_inner(
        &self,
        req: &PictureCompareRequest,
        job: &JobControl,
    ) -> Result<PictureCompareResponse, ApiError> {
        let settings = self.check_picture_words(req)?;
        let target = req.diff_output.as_deref().map(diff_output_target);
        let checkerboard = req.checkerboard || req.view_checkerboard == Some(true);
        // An image never replaces an input (F8).
        if let Some(raw) = &req.diff_output
            && let Some(ReportTarget::File(_)) = &target
        {
            self.refuse_output_onto_inputs("--diff-output", raw, [&req.left, &req.right])?;
        }
        let loaded = self.picture_load_with(req, &settings.decode, job)?;
        let comparison = self.picture_diff(&loaded, &settings.options, job)?;
        let verdict = comparison.verdict(settings.ignore_unimportant, || {
            self.picture_bytes_identical(&loaded, job)
        })?;
        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
        let probes = settings
            .probes
            .iter()
            .map(|&(x, y)| comparison.probe(x, y))
            .collect();
        let mut warnings = Vec::new();
        for (side, picture) in [
            (Side::Left, &loaded.pictures.left),
            (Side::Right, &loaded.pictures.right),
        ] {
            for w in picture.warnings(&of(&loaded.specs, side).canonical()) {
                if req.verbose || !w.code.is_informational() {
                    warnings.push(picture_warning(&w));
                }
            }
        }
        warnings.extend(comparison.warnings().iter().map(picture_warning));
        let mut planned = None;
        let (diff_output, written) = match (&target, &req.diff_output) {
            (Some(target), Some(raw)) if req.dry_run => {
                let area = comparison.area();
                let (line, file, shown) = match target {
                    ReportTarget::File(raw) => {
                        let spec = self.write_spec(raw)?;
                        let verb = match self.router().stat(&spec, hikasa_vfs::FollowLinks::No) {
                            Ok(_) => "replace",
                            Err(_) => "write",
                        };
                        (
                            format!("{verb} {}", spec.canonical()),
                            true,
                            spec.canonical(),
                        )
                    }
                    ReportTarget::Clipboard => (
                        "copy the image to the clipboard".to_owned(),
                        false,
                        raw.to_string_lossy().into_owned(),
                    ),
                    _ => (
                        "write the image to standard output".to_owned(),
                        false,
                        raw.to_string_lossy().into_owned(),
                    ),
                };
                planned = Some(PlannedImage { line, file });
                let info = DiffOutput {
                    target: shown,
                    pane: settings.pane,
                    mode: settings.mode,
                    width: area.width,
                    height: area.height,
                    checkerboard,
                };
                (Some(info), None)
            }
            (Some(target), Some(raw)) => {
                let written = self.picture_write(
                    &comparison,
                    settings.pane,
                    settings.mode,
                    checkerboard,
                    target,
                    job,
                )?;
                let info = DiffOutput {
                    target: match &written.spec {
                        Some(spec) => spec.canonical(),
                        None => raw.to_string_lossy().into_owned(),
                    },
                    pane: settings.pane,
                    mode: settings.mode,
                    width: written.width,
                    height: written.height,
                    checkerboard,
                };
                (Some(info), Some(written))
            }
            _ => (None, None),
        };
        Ok(PictureCompareResponse {
            loaded,
            comparison,
            kind: verdict.kind,
            exit_code: verdict.exit_code,
            mode: settings.mode,
            ignore_unimportant: settings.ignore_unimportant,
            probes,
            diff_output,
            written,
            planned,
            colors: self.picture_colors(),
            warnings,
        })
    }

    /// True when the two inputs of `loaded` hold the same bytes (the verdict's *binary same* of D-339, asked only when
    /// every position is same): the streams compared through the VFS. A job. Additive (M2 W13b: the view asks it once
    /// when a comparison of the pair first lands all same).
    ///
    /// # Errors
    ///
    /// The VFS errors of the inputs; [`ApiError::Cancelled`].
    pub fn picture_bytes_identical(
        &self,
        loaded: &PictureLoaded,
        job: &JobControl,
    ) -> Result<bool, ApiError> {
        let router = self.router();
        let source = |side: Side| -> Result<VfsSource<'_>, ApiError> {
            let spec = of(&loaded.specs, side);
            let entry = self.stat_input(side, spec, job)?;
            Ok(VfsSource::new(&*router, spec.clone(), entry))
        };
        let (l, r) = (source(Side::Left)?, source(Side::Right)?);
        compare_sources(&l, &r, &self.stream_options(), &job.ctl())
            .map(|o| o == hikasa_vfs::BinaryOutcome::Same)
            .map_err(|e| {
                if e.is_cancelled() {
                    ApiError::Cancelled
                } else {
                    ApiError::from_vfs(e)
                }
            })
    }

    /// The response of a comparison a view holds (M2 W13b, D-361): the verdict of D-339 under the view's
    /// `ignore_unimportant` (`bytes_identical` answers *binary same* when every position is same: the view asked
    /// [`Service::picture_bytes_identical`] once; `false` before it knows), the decoders' warnings (informational ones
    /// only with `verbose`) and the geometry's, the view's mode and colours; no probes, no image. What the status bar,
    /// Info, the report dialog and `--wait` read; equal to [`Service::picture_compare`]'s of the same words. Pure.
    pub fn picture_response(
        &self,
        loaded: &PictureLoaded,
        comparison: &Arc<PictureComparison>,
        view: &PictureViewState,
    ) -> PictureCompareResponse {
        let verdict = comparison
            .verdict(view.ignore_unimportant, || {
                Ok::<bool, std::convert::Infallible>(view.bytes_identical)
            })
            .unwrap_or(hikasa_picture::PictureVerdict::ERROR);
        let mut warnings = Vec::new();
        for (side, picture) in [
            (Side::Left, &loaded.pictures.left),
            (Side::Right, &loaded.pictures.right),
        ] {
            for w in picture.warnings(&of(&loaded.specs, side).canonical()) {
                if view.verbose || !w.code.is_informational() {
                    warnings.push(picture_warning(&w));
                }
            }
        }
        warnings.extend(comparison.warnings().iter().map(picture_warning));
        PictureCompareResponse {
            loaded: loaded.clone(),
            comparison: Arc::clone(comparison),
            kind: verdict.kind,
            exit_code: verdict.exit_code,
            mode: view.mode,
            ignore_unimportant: view.ignore_unimportant,
            probes: Vec::new(),
            diff_output: None,
            written: None,
            planned: None,
            colors: view.colors,
            warnings,
        }
    }

    /// Refuses an output that would replace an input (F8): `raw` (a file target) names the same spec as `left` or
    /// `right` (an archive member included), or the same local file (a link or a hard link to it: the same file
    /// id). Standard input and the clipboard are no files. `flag` names the output in the message.
    pub fn refuse_output_onto_inputs(
        &self,
        flag: &str,
        raw: &std::ffi::OsStr,
        inputs: [&std::ffi::OsStr; 2],
    ) -> Result<(), ApiError> {
        let Ok(target) = self.write_spec(raw) else {
            // A target that does not parse is refused by the write itself.
            return Ok(());
        };
        let router = self.router();
        let id = |spec: &Spec| {
            router
                .stat(spec, hikasa_vfs::FollowLinks::Yes)
                .ok()
                .and_then(|e| e.file_id)
        };
        let target_id = id(&target);
        for (side, input) in [("left", inputs[0]), ("right", inputs[1])] {
            if input == "-" {
                continue;
            }
            let Ok(spec) = Spec::parse_os(input, &self.spec_context()) else {
                continue;
            };
            if matches!(spec, Spec::Stdin | Spec::Clipboard) {
                continue;
            }
            let Ok(spec) = self.resolve(spec, self.use_of(input)) else {
                continue;
            };
            let same = spec.canonical() == target.canonical()
                || (target_id.is_some() && id(&spec) == target_id);
            if same {
                return Err(ApiError::InvalidArgument(format!(
                    "{flag} {}: it is the {side} input; an output never replaces an input",
                    target.canonical()
                )));
            }
        }
        Ok(())
    }

    /// The Rules content test of Picture-kind formats (D-344): the tolerance, Auto Scale, tone map and pixel limit of
    /// the Picture Compare defaults session (`sessions/_defaults/picture-compare.toml`, else the built-in defaults);
    /// a defaults session whose words do not parse gives the built-in defaults.
    pub fn picture_comparator(&self) -> PictureComparator {
        self.picture_comparator_with(hikasa_formats::EnabledOverrides::default(), None)
    }

    /// [`Service::picture_comparator`] whose sides convert through their Picture formats' load conversions (F1): each
    /// name's format detected with `overrides`, or the `forced` conversion for both (`quick --format`).
    pub(crate) fn picture_comparator_with(
        &self,
        overrides: hikasa_formats::EnabledOverrides,
        forced: Option<ExternalConversion>,
    ) -> PictureComparator {
        let mut comparator = PictureComparator {
            converter: Some(self.picture_converter(overrides, forced)),
            ..PictureComparator::default()
        };
        if let Some(rules) = self.picture_defaults()
            && let Ok(settings) = picture_settings_of(&rules, &PictureGeometryWords::default())
        {
            comparator.options.tolerance = settings.options.tolerance;
            comparator.options.geometry.auto_scale = settings.options.geometry.auto_scale;
            comparator.decode = settings.decode;
        }
        comparator
    }
}

// ---- sessions (D-349) --------------------------------------------------------------------------------------------

/// `[rules.picture]` of a `picture-compare` session (D-349).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PictureRulesFile {
    /// `tolerance`, `range`, `blend`.
    pub mode: String,
    /// 0…255.
    pub tolerance: u64,
    /// 0…100.
    pub blend: u64,
    /// Auto Scale.
    pub auto_scale: bool,
    /// `clamp`, `reinhard`.
    pub tone_map: String,
    /// 1…4,096.
    pub max_megapixels: u64,
    /// Ignore Unimportant.
    pub ignore_unimportant: bool,
}

impl Default for PictureRulesFile {
    fn default() -> PictureRulesFile {
        PictureRulesFile {
            mode: DiffMode::Tolerance.as_str().to_owned(),
            tolerance: 0,
            blend: u64::from(DiffMode::DEFAULT_BLEND),
            auto_scale: false,
            tone_map: ToneMap::Clamp.as_str().to_owned(),
            max_megapixels: hikasa_picture::codec::DEFAULT_MAX_PIXELS >> 20,
            ignore_unimportant: false,
        }
    }
}

impl PictureRulesFile {
    /// The rule words of the file's values (`--blend` only in blend mode).
    pub fn words(&self) -> PictureRuleWords {
        let blend_mode =
            DiffMode::parse(&self.mode).is_ok_and(|m| matches!(m, DiffMode::Blend { .. }));
        PictureRuleWords {
            mode: Some(self.mode.clone()),
            tolerance: Some(self.tolerance.to_string()),
            blend: blend_mode.then(|| self.blend.to_string()),
            auto_scale: self.auto_scale,
            tone_map: Some(self.tone_map.clone()),
            max_megapixels: Some(self.max_megapixels.to_string()),
            ignore_unimportant: self.ignore_unimportant,
        }
    }

    /// The file's values of rule words (absent words: the defaults).
    pub fn of_words(words: &PictureRuleWords) -> Result<PictureRulesFile, ApiError> {
        let s = picture_settings_of(words, &PictureGeometryWords::default())?;
        Ok(PictureRulesFile {
            mode: s.mode.as_str().to_owned(),
            tolerance: u64::from(s.options.tolerance),
            blend: match s.mode {
                DiffMode::Blend { percent } => u64::from(percent),
                _ => u64::from(DiffMode::DEFAULT_BLEND),
            },
            auto_scale: s.options.geometry.auto_scale,
            tone_map: s.decode.tone_map.as_str().to_owned(),
            max_megapixels: s.decode.max_pixels >> 20,
            ignore_unimportant: s.ignore_unimportant,
        })
    }
}

/// The rules of a `picture-compare` session (`[rules]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PictureSessionRules {
    /// `[rules.picture]`.
    pub picture: PictureRulesFile,
}

/// The geometry words of a session's `[view]`: `orientation_left`, `orientation_right`, `offset = [dx, dy]`.
pub fn picture_geometry_words_of(view: &toml::Table) -> PictureGeometryWords {
    let word = |key: &str| super::hex::view_word(view, key);
    PictureGeometryWords {
        left_orientation: word("orientation_left"),
        right_orientation: word("orientation_right"),
        offset: match view.get("offset") {
            Some(toml::Value::Array(a)) => Some(
                a.iter()
                    .map(|v| match v {
                        toml::Value::Integer(n) => n.to_string(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            Some(other) => Some(other.to_string()),
            None => None,
        },
    }
}

/// Checks a picture session's words through the parsers (`session edit --set`, D-349).
pub(crate) fn check_picture_session(
    rules: &PictureSessionRules,
    view: &toml::Table,
) -> Result<(), ApiError> {
    let r = &rules.picture;
    let mut words = r.words();
    // The blend percentage is checked in any mode (the file keeps it for blend mode).
    words::parse_blend(&r.blend.to_string()).map_err(vocab)?;
    words.blend = None;
    picture_settings_of(&words, &picture_geometry_words_of(view))?;
    if let Some(toml::Value::Boolean(_)) | None = view.get("checkerboard") {
    } else {
        return Err(ApiError::InvalidArgument(
            "invalid view.checkerboard: expected true or false".to_owned(),
        ));
    }
    if let Some(layout) = super::hex::view_word(view, "layout")
        && !matches!(layout.as_str(), "side-by-side" | "stacked")
    {
        return Err(ApiError::InvalidArgument(format!(
            "invalid view.layout {layout:?}: expected side-by-side, stacked"
        )));
    }
    picture_view_keys_of(view)?;
    Ok(())
}

/// The view-only keys of a picture session's `[view]` that M2 W13b added (D-370): `zoom` (`fit`, `actual` or a
/// percentage, [`hikasa_picture::parse_zoom`]; default `fit`), `center` (`[x, y]` in area pixels, finite; written only
/// when the zoom is not `fit`) and `difference_pane` (default true).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureViewKeys {
    /// The zoom.
    pub zoom: hikasa_picture::Zoom,
    /// The area point at the panes' centres.
    pub center: Option<(f64, f64)>,
    /// The difference pane is shown.
    pub difference_pane: bool,
}

/// The `[view]` table of a picture view's session (D-349, D-370; M2 W13b): `orientation_left`, `orientation_right`,
/// `offset = [dx, dy]`, `checkerboard`, `layout` (`side-by-side`, `stacked`), `zoom`, `center` (only when the zoom is
/// not `fit`) and `difference_pane`. What [`picture_geometry_words_of`] and [`picture_view_keys_of`] read back. Pure.
pub fn picture_view_table(
    geometry: &Geometry,
    keys: &PictureViewKeys,
    checkerboard: bool,
    stacked: bool,
) -> toml::Table {
    let mut t = toml::Table::new();
    let word = |s: &str| toml::Value::String(s.to_owned());
    t.insert(
        "orientation_left".to_owned(),
        word(geometry.orientation.left.as_str()),
    );
    t.insert(
        "orientation_right".to_owned(),
        word(geometry.orientation.right.as_str()),
    );
    t.insert(
        "offset".to_owned(),
        toml::Value::Array(vec![
            toml::Value::Integer(i64::from(geometry.offset.0)),
            toml::Value::Integer(i64::from(geometry.offset.1)),
        ]),
    );
    t.insert(
        "checkerboard".to_owned(),
        toml::Value::Boolean(checkerboard),
    );
    t.insert(
        "layout".to_owned(),
        word(if stacked { "stacked" } else { "side-by-side" }),
    );
    t.insert("zoom".to_owned(), toml::Value::String(keys.zoom.word()));
    if keys.zoom != hikasa_picture::Zoom::Fit
        && let Some((x, y)) = keys.center
    {
        t.insert(
            "center".to_owned(),
            toml::Value::Array(vec![toml::Value::Float(x), toml::Value::Float(y)]),
        );
    }
    t.insert(
        "difference_pane".to_owned(),
        toml::Value::Boolean(keys.difference_pane),
    );
    t
}

/// Reads and checks [`PictureViewKeys`] of a session's `[view]` (`session edit --set` refuses what this refuses; a
/// view reading its session takes the defaults of a refused key).
///
/// # Errors
///
/// `InvalidArgument` naming the key.
pub fn picture_view_keys_of(view: &toml::Table) -> Result<PictureViewKeys, ApiError> {
    let zoom = match view.get("zoom") {
        None => hikasa_picture::Zoom::Fit,
        Some(toml::Value::String(w)) => hikasa_picture::parse_zoom(w)
            .map_err(|e| ApiError::InvalidArgument(format!("invalid view.zoom: {e}")))?,
        Some(other) => {
            return Err(ApiError::InvalidArgument(format!(
                "invalid view.zoom {other}: expected fit, actual or a percentage such as \"150%\""
            )));
        }
    };
    let number = |v: &toml::Value| match v {
        toml::Value::Integer(n) => Some(*n as f64),
        toml::Value::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    };
    let center = match view.get("center") {
        None => None,
        Some(toml::Value::Array(a)) if a.len() == 2 => match (number(&a[0]), number(&a[1])) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
        .map(Some)
        .ok_or_else(|| {
            ApiError::InvalidArgument(
                "invalid view.center: expected [x, y] with numbers".to_owned(),
            )
        })?,
        Some(_) => {
            return Err(ApiError::InvalidArgument(
                "invalid view.center: expected [x, y] with numbers".to_owned(),
            ));
        }
    };
    let difference_pane = match view.get("difference_pane") {
        None => true,
        Some(toml::Value::Boolean(b)) => *b,
        Some(_) => {
            return Err(ApiError::InvalidArgument(
                "invalid view.difference_pane: expected true or false".to_owned(),
            ));
        }
    };
    Ok(PictureViewKeys {
        zoom,
        center,
        difference_pane,
    })
}

impl Service {
    /// The rule words `picture compare` runs with (D-374, W12c review fixes F10): the words typed on top of the
    /// Picture Compare defaults session, as `session new --type picture-compare` starts from it and the folder content
    /// test compares with it (D-344). A word typed wins; an absent one is the session's (`--blend` only when the mode
    /// is not typed either); `--auto-scale` and `--ignore-unimportant` add to the session's. A defaults session that
    /// cannot be read leaves the words as typed.
    pub fn picture_rules_on_defaults(&self, typed: &PictureRuleWords) -> PictureRuleWords {
        let Some(defaults) = self.picture_defaults() else {
            return typed.clone();
        };
        PictureRuleWords {
            mode: typed.mode.clone().or(defaults.mode),
            tolerance: typed.tolerance.clone().or(defaults.tolerance),
            blend: match &typed.mode {
                Some(_) => typed.blend.clone(),
                None => typed.blend.clone().or(defaults.blend),
            },
            auto_scale: typed.auto_scale || defaults.auto_scale,
            tone_map: typed.tone_map.clone().or(defaults.tone_map),
            max_megapixels: typed.max_megapixels.clone().or(defaults.max_megapixels),
            ignore_unimportant: typed.ignore_unimportant || defaults.ignore_unimportant,
        }
    }

    /// The rule words of the Picture Compare defaults session (`None`: it cannot be read).
    fn picture_defaults(&self) -> Option<PictureRuleWords> {
        let session = self
            .session_defaults(crate::sessions::SessionType::PictureCompare)
            .ok()?;
        match &session.rules {
            crate::sessions::SessionRules::Picture(rules) => Some(rules.picture.words()),
            _ => None,
        }
    }
}
