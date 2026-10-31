//! `format list/show/detect/tokens` and `vfs ls/stat/cat` (§3.5, §4.3.4).

use std::ffi::OsStr;
use std::io::{Read, Write};

use hikasa_core::{ErrorReport, Phase, Progress, Side, Spec, Unit};
use hikasa_formats::{
    DetectReason, Detection, EnabledOverrides, FormatError, LineState, Origin, ShadowStatus,
    to_toml, tokenize_all,
};
use hikasa_report::json::{DetectReasonJson, FormatJson};
use hikasa_text::{LineIdx, load_document};
use hikasa_vfs::{FollowLinks, ListOptions, SpecUse, Vfs, VfsError, VfsOp};

use super::dto::{
    EncodingDto, FileFormatDto, FormatDetectDto, FormatListDto, FormatListItemDto, FormatRefDto,
    FormatShowDto, FormatTokensDto, LineStateDto, ShadowStatusDto, TokenDto, TokenLineDto,
    VfsCatDto, VfsEntryDto, VfsListDto, VfsListErrorDto, VfsStatDto,
};
use super::requests::FormatTokensRequest;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// Bytes per encoded token in the `format tokens` arena: start (u32), end (u32), element (u16), item (u16).
const TOKEN_BYTES: usize = 12;

/// `item` value meaning "no grammar item" in the token arena.
const NO_ITEM: u16 = u16::MAX;

/// Buffer of `vfs cat` (a cancellation check per buffer).
const CAT_BUFFER: usize = 1 << 20;

impl Service {
    /// `format list`: every registry entry in detection order with its origin, enabled flag and shadowing
    /// status (§2.12.3). The skipped user formats are in [`Service::startup_warnings`].
    pub fn format_list(&self) -> FormatListDto {
        let registry = self.registry();
        let statuses = registry.shadow_report(&EnabledOverrides::default());
        let formats = registry
            .entries()
            .iter()
            .zip(statuses)
            .enumerate()
            .map(|(k, (entry, (_, status)))| {
                let (origin, path) = origin_of(&entry.origin);
                FormatListItemDto {
                    position: u32::try_from(k + 1).unwrap_or(u32::MAX),
                    id: entry.id().as_str().to_owned(),
                    name: entry.format.name.clone(),
                    kind: entry.format.kind.as_str(),
                    mask: entry.format.mask.clone(),
                    enabled: entry.enabled,
                    origin,
                    path,
                    status: shadow_dto(status),
                }
            })
            .collect();
        FormatListDto {
            config_dir: self.config().dir.display().to_string(),
            formats,
        }
    }

    /// `format show`: the entry (by id, then by case-insensitive name) with its full definition and, when it
    /// does not compile, the compile error.
    pub fn format_show(&self, id_or_name: &str) -> Result<FormatShowDto, ApiError> {
        run_guarded(|| {
            let registry = self.registry();
            let entry = registry
                .find(id_or_name)
                .ok_or_else(|| FormatError::UnknownFormat {
                    name: id_or_name.to_owned(),
                })?;
            let (origin, path) = origin_of(&entry.origin);
            let compile_error = registry
                .compiled(entry.id())
                .err()
                .map(|e| ErrorReport::capture(&e, None));
            Ok(FormatShowDto {
                id: entry.id().as_str().to_owned(),
                origin,
                path,
                enabled: entry.enabled,
                format: FileFormatDto::new(&entry.format),
                compile_error,
            })
        })
    }

    /// The canonical TOML of a format (the human output of `format show`; additive).
    pub fn format_toml(&self, id_or_name: &str) -> Result<String, ApiError> {
        let registry = self.registry();
        let entry = registry
            .find(id_or_name)
            .ok_or_else(|| FormatError::UnknownFormat {
                name: id_or_name.to_owned(),
            })?;
        Ok(to_toml(&entry.format))
    }

    /// `format detect` (a "-" file uses the stdin name hint, D-15).
    ///
    /// Detection uses the name only (the file is not read; `-` does not read standard input, its name is the
    /// `--stdin-name` hint). A spec without a name (stdin without a hint) is "Everything Else" (`fallback`).
    pub fn format_detect(&self, file: &OsStr) -> Result<FormatDetectDto, ApiError> {
        run_guarded(|| {
            let spec = hikasa_core::Spec::parse_os(file, &self.spec_context())?;
            let detection = self.detect_name(self.detection_name(&spec).as_deref())?;
            let format = &detection.format;
            Ok(FormatDetectDto {
                file: spec.canonical(),
                format: FormatRefDto {
                    id: format.id().map(|id| id.as_str().to_owned()),
                    name: format.name().to_owned(),
                    kind: format.kind().as_str(),
                },
                reason: DetectReasonJson::new(&detection.reason),
            })
        })
    }

    /// `format tokens`: loads the file (the format's conversion and encoding rules, `--encoding`), tokenises
    /// every line on the service pool (`tokenize_all`, so multi-line state is exact) and returns the tokens of
    /// the selected lines.
    pub fn format_tokens(
        &self,
        req: &FormatTokensRequest,
        job: &JobControl,
    ) -> Result<FormatTokensDto, ApiError> {
        run_guarded(|| self.format_tokens_inner(req, job))
    }

    fn format_tokens_inner(
        &self,
        req: &FormatTokensRequest,
        job: &JobControl,
    ) -> Result<FormatTokensDto, ApiError> {
        let spec = self.parse_spec(&req.file)?;
        let detection = match req.format.as_deref() {
            Some(forced) => self.registry().force(forced)?,
            None => self.detect_name(self.detection_name(&spec).as_deref())?,
        };
        let load = self.load_options(req.encoding.as_deref())?;
        let ctl = job.ctl();
        let (doc, raw) = load_document(
            &*self.router(),
            &spec,
            &detection.format,
            &load,
            Some(self.external()),
            &ctl,
        )
        .map_err(|e| ApiError::from_text(e, Side::Left, &spec.canonical()))?;
        drop(raw);
        let count = doc.line_count();
        // 0-based [first, last]; an empty selection (an empty range, line 0, a range past the end, an empty
        // document) selects nothing, never line 1.
        let selection = match &req.lines {
            Some(range) if range.is_empty() || *range.end() == 0 => None,
            Some(range) => {
                let first = range.start().saturating_sub(1);
                let last = (range.end() - 1).min(count.saturating_sub(1));
                (count > 0 && first <= last).then_some((first, last))
            }
            None => (count > 0).then(|| (0, count - 1)),
        };
        let selected =
            |line: u32| selection.is_some_and(|(first, last)| line >= first && line <= last);
        let grammar = detection.format.grammar();
        ctl.progress_now(Progress {
            phase: Phase::Tokenizing,
            done: 0,
            total: Some(u64::from(count)),
            unit: Unit::Lines,
            current: None,
        });
        let tokenized = tokenize_all(
            grammar,
            &doc,
            Some(self.pool()),
            &job.cancel,
            |line, _text, tokens, out| {
                if selected(line) {
                    for token in tokens {
                        out.extend_from_slice(&token.start.to_le_bytes());
                        out.extend_from_slice(&token.end.to_le_bytes());
                        out.extend_from_slice(&token.element.0.to_le_bytes());
                        out.extend_from_slice(&token.item.unwrap_or(NO_ITEM).to_le_bytes());
                    }
                }
            },
        )
        .map_err(|_| ApiError::Cancelled)?;
        ctl.progress_now(Progress {
            phase: Phase::Tokenizing,
            done: u64::from(count),
            total: Some(u64::from(count)),
            unit: Unit::Lines,
            current: None,
        });
        let elements = grammar.elements();
        let mut lines = Vec::new();
        if let Some((first, last)) = selection {
            for line in first..=last {
                job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                let text = doc.line(LineIdx(line));
                let state_in = tokenized
                    .start_states
                    .get(line as usize)
                    .copied()
                    .unwrap_or(LineState::Normal);
                let tokens = tokenized
                    .bytes
                    .get(line)
                    .as_chunks::<TOKEN_BYTES>()
                    .0
                    .iter()
                    .map(|b| decode_token(b, &text, elements))
                    .collect();
                lines.push(TokenLineDto {
                    line: line + 1,
                    state_in: LineStateDto::new(state_in),
                    tokens,
                });
            }
        }
        let encoding = doc.encoding();
        Ok(FormatTokensDto {
            file: spec.canonical(),
            format: FormatJson::new(&detection.format, Some(&detection.reason)),
            encoding: EncodingDto {
                name: encoding.encoding.name(),
                source: encoding.source,
                bom: encoding.bom_len > 0,
                malformed: doc.malformed(),
            },
            regex_errors: tokenized.regex_errors,
            lines,
        })
    }

    /// `vfs ls`: the folder's entries sorted by name bytes, and the entries whose metadata failed.
    pub fn vfs_ls(
        &self,
        spec: &OsStr,
        follow: FollowLinks,
        job: &JobControl,
    ) -> Result<VfsListDto, ApiError> {
        run_guarded(|| {
            // `vfs ls a.zip` lists the members (§1.5.13).
            let spec = self.parse_spec_in(spec, SpecUse::Folder, &job.cancel)?;
            let options = ListOptions {
                follow,
                prefilter: None,
            };
            let mut listing = self
                .vfs()
                .list(&spec, &options, &job.cancel)
                .map_err(ApiError::from_vfs)?;
            listing.sort_by_name();
            Ok(VfsListDto {
                spec: spec.canonical(),
                entries: listing.entries.iter().map(VfsEntryDto::new).collect(),
                errors: listing
                    .errors
                    .iter()
                    .map(|e| VfsListErrorDto {
                        name: e.name.as_str().to_owned(),
                        error: e.error.clone(),
                    })
                    .collect(),
            })
        })
    }

    /// `vfs stat`.
    pub fn vfs_stat(&self, spec: &OsStr, follow: FollowLinks) -> Result<VfsStatDto, ApiError> {
        run_guarded(|| self.stat_dto(&self.parse_spec(spec)?, follow))
    }

    /// `vfs stat` inside a job: indexing an archive stops with it.
    pub fn vfs_stat_in(
        &self,
        spec: &OsStr,
        follow: FollowLinks,
        job: &JobControl,
    ) -> Result<VfsStatDto, ApiError> {
        run_guarded(|| {
            let spec = self.parse_spec_in(spec, self.use_of(spec), &job.cancel)?;
            self.vfs()
                .prepare(&spec, &job.ctl())
                .map_err(ApiError::from_vfs)?;
            self.stat_dto(&spec, follow)
        })
    }

    fn stat_dto(&self, spec: &Spec, follow: FollowLinks) -> Result<VfsStatDto, ApiError> {
        let entry = self.vfs().stat(spec, follow).map_err(ApiError::from_vfs)?;
        let caps = self.vfs().capabilities(spec).map_err(ApiError::from_vfs)?;
        Ok(VfsStatDto {
            spec: spec.canonical(),
            entry: VfsEntryDto::new(&entry),
            capabilities: VfsStatDto::capability_names(caps),
        })
    }

    /// `vfs cat`: copies content into `out`; returns bytes copied.
    ///
    /// Cancellation is checked before every 1 MiB buffer; progress is `reading` (bytes). A failure writing to
    /// `out` is `Vfs(Io { spec: "<output>" })` ([`ApiError::is_broken_pipe`] detects a closed pipe).
    pub fn vfs_cat(
        &self,
        spec: &OsStr,
        out: &mut dyn Write,
        job: &JobControl,
    ) -> Result<u64, ApiError> {
        run_guarded(|| {
            let spec = self.parse_spec_in(spec, self.use_of(spec), &job.cancel)?;
            let canonical = spec.canonical();
            // Indexing runs here, where it can be cancelled; `stat` below cannot.
            self.vfs()
                .prepare(&spec, &job.ctl())
                .map_err(ApiError::from_vfs)?;
            let total = self
                .vfs()
                .stat(&spec, FollowLinks::Yes)
                .ok()
                .map(|e| e.size);
            let mut reader = self
                .vfs()
                .open_read_with(&spec, &job.cancel)
                .map_err(ApiError::from_vfs)?;
            let mut buffer = vec![0u8; CAT_BUFFER];
            let mut copied: u64 = 0;
            let ctl = job.ctl();
            loop {
                job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                let n = match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        return Err(ApiError::from_vfs(VfsError::from_io(
                            canonical.as_str(),
                            VfsOp::Read,
                            e,
                        )));
                    }
                };
                out.write_all(buffer.get(..n).unwrap_or_default())
                    .map_err(ApiError::output)?;
                copied += n as u64;
                ctl.progress(Phase::Reading, || Progress {
                    phase: Phase::Reading,
                    done: copied,
                    total,
                    unit: Unit::Bytes,
                    current: None,
                });
            }
            out.flush().map_err(ApiError::output)?;
            ctl.progress_now(Progress {
                phase: Phase::Reading,
                done: copied,
                total: Some(copied),
                unit: Unit::Bytes,
                current: None,
            });
            Ok(copied)
        })
    }

    /// `vfs cat --json` (D-32): the content as `hikasa.vfs_cat.v1` (base64; the content is buffered). Additive.
    pub fn vfs_cat_dto(&self, spec: &OsStr, job: &JobControl) -> Result<VfsCatDto, ApiError> {
        let mut bytes = Vec::new();
        self.vfs_cat(spec, &mut bytes, job)?;
        let canonical = self.parse_spec(spec)?.canonical();
        Ok(VfsCatDto::new(canonical, &bytes))
    }

    /// Detection by name (D-15): `None` (no name) is "Everything Else".
    fn detect_name(&self, name: Option<&str>) -> Result<Detection, ApiError> {
        match name {
            Some(name) => Ok(self.registry().detect(name, &EnabledOverrides::default())?),
            None => Ok(Detection {
                format: std::sync::Arc::clone(self.registry().everything_else()),
                reason: DetectReason::Fallback,
            }),
        }
    }
}

/// `(origin name, user path)` of a registry entry.
fn origin_of(origin: &Origin) -> (&'static str, Option<String>) {
    match origin {
        Origin::BuiltIn => ("built_in", None),
        Origin::User { path } => ("user", Some(path.display().to_string())),
    }
}

/// The DTO of a shadowing status.
fn shadow_dto(status: ShadowStatus) -> ShadowStatusDto {
    match status {
        ShadowStatus::Visible => ShadowStatusDto {
            status: "visible",
            by: Vec::new(),
        },
        ShadowStatus::Shadowed { by } => ShadowStatusDto {
            status: "shadowed",
            by: by.into_iter().map(|id| id.as_str().to_owned()).collect(),
        },
        ShadowStatus::ManualOnly => ShadowStatusDto {
            status: "manual_only",
            by: Vec::new(),
        },
        ShadowStatus::Disabled => ShadowStatusDto {
            status: "disabled",
            by: Vec::new(),
        },
    }
}

/// Decodes one token of the arena written by `format_tokens`.
fn decode_token(
    bytes: &[u8; TOKEN_BYTES],
    line: &str,
    elements: &hikasa_formats::ElementTable,
) -> TokenDto {
    let [s0, s1, s2, s3, e0, e1, e2, e3, l0, l1, i0, i1] = *bytes;
    let (start, end) = (
        u32::from_le_bytes([s0, s1, s2, s3]),
        u32::from_le_bytes([e0, e1, e2, e3]),
    );
    let element = hikasa_formats::ElementId(u16::from_le_bytes([l0, l1]));
    let item = u16::from_le_bytes([i0, i1]);
    TokenDto {
        start,
        end,
        element: elements.name(element).to_owned(),
        item: (item != NO_ITEM).then_some(item),
        text: line
            .get(start as usize..end as usize)
            .unwrap_or_default()
            .to_owned(),
    }
}
