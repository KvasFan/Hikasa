//! The format editor's calls (M2 W11c; contract §1.11.26, §1.11.27, D-318, D-319): the typed forms of `format show`
//! (a format to edit, with the stamp of its file), `format edit --from-file` ([`Service::format_save`]), the compile
//! check of docs/10 §1.1 ([`Service::format_check`], its message naming the grammar item) and `format tokens` on a
//! sample ([`Service::format_preview`]). No `hikasa` subcommand is added: each is a form of an existing one.
//!
//! ```text
//! format_for_edit(id)          = the registry read now: the format, its origin, the stamp of formats/<id>.toml
//! format_check(f)              = parse_format(to_toml(f)) + CompiledFormat::compile(f)
//! format_save(id, f, expected) = format_check(f); expected ≠ stamp now → Conflict;
//!                                write to_toml(f) (+ the unknown keys of the file it replaces, D-70)
//! format_sample(path, f)       = the first 64 KiB of the file decoded as a document of f (BOM out, whole characters)
//! format_preview(f, sample)    = the sample's lines tokenized with f (tokenize_all), as `format tokens` lists them
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::Warning;
use hikasa_formats::{
    CompiledFormat, FileFormat, FormatError, FormatRegistry, KeyStep, LineState, Origin, decode,
    decode_prefix, detect, parse_format, to_toml, tokenize_all,
};
use toml::{Table, Value};

use super::dto::{LineStateDto, TokenLineDto};
use super::info::{NO_ITEM, TOKEN_BYTES, decode_token};
use super::settings::{FormatWriteDto, writable_format_id, write_dto};
use super::{ApiError, Service};
use crate::error::SessionError;
use crate::jobs::{JobControl, run_guarded};
use crate::settings::keys::KeySegment;
use crate::settings::{ChangeStamp, tree};

/// The bytes of a file the preview's sample takes (§1.11.27: the first 64 KiB).
pub const PREVIEW_SAMPLE_BYTES: usize = 64 * 1024;

/// One line of a preview: the tokens of `format tokens` (same DTO, same element names and item numbers).
pub type PreviewLine = TokenLineDto;

/// A format as the editor reads it (§1.11.27).
#[derive(Debug, Clone, PartialEq)]
pub struct FormatForEdit {
    /// The definition.
    pub format: FileFormat,
    /// `built_in`, `user` or `override` (a user file with a built-in's id).
    pub origin: &'static str,
    /// `formats/<id>.toml`, the file a save writes (a built-in gets its override file there).
    pub path: PathBuf,
    /// The stamp of that file when it was read (`Absent` for a built-in without an override): a save checks it.
    pub stamp: ChangeStamp,
    /// The id is a built-in's.
    pub builtin: bool,
}

/// The message of a format's error as the editor shows it: a field below `grammar[k]` names *Grammar item k+1*, one
/// below `line_weight[k]` *Line weight k+1* (1-based, as the lists number them).
fn editor_message(e: &FormatError) -> String {
    let (field, reason) = match e {
        FormatError::Invalid { field, reason, .. } => (field.clone(), reason.clone()),
        FormatError::Pattern {
            location, source, ..
        } => (location.clone(), source.to_string()),
        other => return other.to_string(),
    };
    for (prefix, caption) in [
        ("grammar[", "Grammar item"),
        ("line_weight[", "Line weight"),
    ] {
        if let Some(rest) = field.strip_prefix(prefix)
            && let Some((index, tail)) = rest.split_once(']')
            && let Ok(k) = index.parse::<usize>()
        {
            let tail = tail.trim_start_matches('.');
            return if tail.is_empty() {
                format!("{caption} {}: {reason}", k + 1)
            } else {
                format!("{caption} {}: {tail}: {reason}", k + 1)
            };
        }
    }
    format!("{field}: {reason}")
}

/// The lines of a sample as a document reads them: `\r\n`, `\n` and `\r` end a line; a final line break adds no
/// empty line.
fn sample_lines(sample: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sample;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(at) => {
                out.push(rest[..at].to_owned());
                let eol = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[at + eol..];
            }
            None => {
                out.push(rest.to_owned());
                rest = "";
            }
        }
    }
    out
}

impl Service {
    /// The format of `id` (or name) as the editor reads it, from the registry read now (the format write commands never
    /// use the startup registry): the definition, its origin and the stamp of `formats/<id>.toml`.
    pub fn format_for_edit(&self, id: &str) -> Result<FormatForEdit, ApiError> {
        run_guarded(|| {
            let (store, registry) = self.fresh_formats()?;
            let entry = registry
                .find(id)
                .ok_or_else(|| ApiError::NotFound(format!("unknown format {id:?}")))?;
            let fid = entry.id().as_str().to_owned();
            let builtin = FormatRegistry::builtin()?.get(entry.id()).is_some();
            let origin = match (&entry.origin, builtin) {
                (Origin::BuiltIn, _) => "built_in",
                (Origin::User { .. }, true) => "override",
                (Origin::User { .. }, false) => "user",
            };
            let path = store.user_path(&fid);
            let stamp = ChangeStamp::of(&path)?;
            Ok(FormatForEdit {
                format: (*entry.format).clone(),
                origin,
                path,
                stamp,
                builtin,
            })
        })
    }

    /// The ids of the built-in formats (the manager's *override* origin and *Restore Built-in*).
    pub fn format_builtin_ids(&self) -> Vec<String> {
        FormatRegistry::builtin()
            .map(|r| {
                r.entries()
                    .iter()
                    .map(|e| e.id().as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Checks a format as `format edit` would before writing it (docs/10 §1.1): it reads back from its canonical TOML
    /// with no unknown key and it compiles (patterns within the regex limits, masks).
    /// `invalid_input` whose message names the item (*Grammar item 3: …*); the warnings are the reader's. (A typed item
    /// always reads back from its TOML: the compile is what names it.)
    pub fn format_check(&self, format: &FileFormat) -> Result<Vec<Warning>, ApiError> {
        run_guarded(|| self.checked_format(format).map(|(_, warnings)| warnings))
    }

    /// [`Service::format_check`] with the format as read back from its canonical TOML.
    fn checked_format(&self, format: &FileFormat) -> Result<(FileFormat, Vec<Warning>), ApiError> {
        let (parsed, warnings) = parse_format(&to_toml(format), &format!("format {}", format.id))
            .map_err(|e| ApiError::InvalidArgument(editor_message(&e)))?;
        CompiledFormat::compile(Arc::new(parsed.clone()), self.registry().limits())
            .map_err(|e| ApiError::InvalidArgument(editor_message(&e)))?;
        Ok((parsed, warnings))
    }

    /// Writes an edited format as `format edit --from-file` writes it (D-75; §1.11.27): `formats/<id>.toml` holds the
    /// canonical TOML (a built-in gets its override file; the id is the file's). The format is checked first
    /// ([`Service::format_check`]); a file changed on disk since `expected` was taken is `Conflict` and nothing is
    /// written. The unknown keys of the file it replaces are kept (D-70); comments are not.
    pub fn format_save(
        &self,
        id: &str,
        format: &FileFormat,
        expected: Option<ChangeStamp>,
    ) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = writable_format_id(id)?;
            if format.id != fid {
                return Err(ApiError::InvalidArgument(format!(
                    "the format id is `{}`, not `{id}`",
                    format.id
                )));
            }
            let (store, registry) = self.fresh_formats()?;
            if registry.get(&fid).is_none() {
                return Err(ApiError::NotFound(format!("unknown format `{id}`")));
            }
            let (parsed, warnings) = self.checked_format(format)?;
            if let Some(warning) = warnings.first() {
                return Err(ApiError::InvalidArgument(warning.message.clone()));
            }
            let path = store.user_path(id);
            if let Some(expected) = expected
                && ChangeStamp::of(&path)? != expected
            {
                return Err(SessionError::Conflict {
                    path: path.display().to_string(),
                }
                .into());
            }
            let canonical = to_toml(&parsed);
            let bytes = match unknown_of(&path, &canonical) {
                Some(merged) => merged,
                None => canonical,
            };
            let written = store.write_user_file(id, bytes.as_bytes())?;
            Ok(write_dto("edit", id, vec![written], Vec::new()))
        })
    }

    /// The first [`PREVIEW_SAMPLE_BYTES`] of a file as the preview's sample, decoded as a document of `format` reads
    /// it (W11c review fix F3): its encoding detected with the format's encoding setting, a BOM left out, the Ctrl-Z
    /// rule applied, and the text cut at the last whole character within the bound. So the preview of a whole file
    /// equals `format tokens` of it (a conversion is not run on the sample).
    pub fn format_sample(
        &self,
        path: &std::path::Path,
        format: &FileFormat,
    ) -> Result<String, ApiError> {
        run_guarded(|| {
            use std::io::Read as _;
            let file = std::fs::File::open(path).map_err(|e| SessionError::io(path, e))?;
            let mut bytes = Vec::new();
            // One byte past the bound tells a cut sample from a file of exactly that size.
            file.take(PREVIEW_SAMPLE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| SessionError::io(path, e))?;
            let mut cut = bytes.len() > PREVIEW_SAMPLE_BYTES;
            bytes.truncate(PREVIEW_SAMPLE_BYTES);
            let options = self.load_options(None)?.decode;
            let mut detected = detect(&bytes, &format.encoding, &options)?;
            if format.text.ctrl_z_eof
                && detected.encoding.is_ascii_compatible()
                && let Some(at) = bytes.iter().position(|&b| b == 0x1A)
            {
                bytes.truncate(at);
                cut = false;
                detected = detect(&bytes, &format.encoding, &options)?;
            }
            let decoded = if cut {
                decode_prefix(&bytes, &detected)
            } else {
                decode(&bytes, &detected)
            };
            Ok(decoded.text)
        })
    }

    /// The sample's lines tokenized with the edited format (§1.11.27, the format editor's preview; a job): the tokens
    /// `format tokens` lists for the same lines with that format. A format that does not compile is `invalid_input`
    /// (core's message naming the item); a cancel is `Cancelled`.
    pub fn format_preview(
        &self,
        format: &FileFormat,
        sample: &str,
        job: &JobControl,
    ) -> Result<Vec<PreviewLine>, ApiError> {
        run_guarded(|| {
            let compiled =
                CompiledFormat::compile(Arc::new(format.clone()), self.registry().limits())
                    .map_err(|e| ApiError::InvalidArgument(editor_message(&e)))?;
            let lines = sample_lines(sample);
            let grammar = compiled.grammar();
            let tokenized = tokenize_all(
                grammar,
                &lines,
                Some(self.pool()),
                &job.cancel,
                |_, _, tokens, out| {
                    for token in tokens {
                        out.extend_from_slice(&token.start.to_le_bytes());
                        out.extend_from_slice(&token.end.to_le_bytes());
                        out.extend_from_slice(&token.element.0.to_le_bytes());
                        out.extend_from_slice(&token.item.unwrap_or(NO_ITEM).to_le_bytes());
                    }
                },
            )
            .map_err(|_| ApiError::Cancelled)?;
            let elements = grammar.elements();
            let mut out = Vec::with_capacity(lines.len());
            for (k, text) in lines.iter().enumerate() {
                let line = u32::try_from(k).unwrap_or(u32::MAX);
                let state_in = tokenized
                    .start_states
                    .get(k)
                    .copied()
                    .unwrap_or(LineState::Normal);
                let tokens = tokenized
                    .bytes
                    .get(line)
                    .as_chunks::<TOKEN_BYTES>()
                    .0
                    .iter()
                    .map(|b| decode_token(b, text, elements))
                    .collect();
                out.push(TokenLineDto {
                    line: line + 1,
                    state_in: LineStateDto::new(state_in),
                    tokens,
                });
            }
            Ok(out)
        })
    }
}

/// The canonical TOML with the keys of the file at `path` that the format model does not know (D-70), when there are
/// any; `None` when the file is absent, unreadable, or has none.
///
/// The unknown keys are the model's schema's ([`hikasa_formats::unknown_key_paths`]), never "a key of the old file
/// that the canonical TOML lacks": the canonical writer leaves out empty and default values (no grammar, no element
/// settings, a table flag off), so a setting the user removed would come back (W11c review fix F1). An unknown key
/// goes back only where its table still is (a removed grammar item, element setting or table section takes its
/// unknown keys with it; `[table.regional]`, left out at its defaults, is written for them) and never over a key the
/// canonical TOML holds.
fn unknown_of(path: &std::path::Path, canonical: &str) -> Option<String> {
    let old: Table = std::fs::read_to_string(path).ok()?.parse().ok()?;
    let unknown = hikasa_formats::unknown_key_paths(&old);
    if unknown.is_empty() {
        return None;
    }
    let mut known = Value::Table(canonical.parse::<Table>().ok()?);
    let mut merged = false;
    for steps in unknown {
        let path: Vec<KeySegment> = steps
            .into_iter()
            .map(|step| match step {
                KeyStep::Key(key) => KeySegment::Name(key),
                KeyStep::Index(k) => KeySegment::Index(k),
            })
            .collect();
        let Some((last, parent)) = path.split_last() else {
            continue;
        };
        let Some(value) = tree::get_in(&old, &path) else {
            continue;
        };
        let room = match tree::get(&known, parent) {
            Some(Value::Table(t)) => !t.contains_key(&last.as_key()),
            // The regional settings at their defaults are not written; their table goes back with its unknown keys.
            None => {
                let table = KeySegment::Name("table".into());
                parent == [table.clone(), KeySegment::Name("regional".into())]
                    && matches!(tree::get(&known, &[table]), Some(Value::Table(_)))
            }
            Some(_) => false,
        };
        if room && tree::set(&mut known, &path, value.clone()).is_ok() {
            merged = true;
        }
    }
    let Value::Table(t) = known else {
        return None;
    };
    if !merged {
        return None;
    }
    toml::to_string(&t).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_item() {
        let e = FormatError::Invalid {
            id: "x".into(),
            field: "grammar[2].to_column".into(),
            reason: "must be ≥ from_column".into(),
        };
        assert_eq!(
            editor_message(&e),
            "Grammar item 3: to_column: must be ≥ from_column"
        );
        let e = FormatError::Invalid {
            id: "x".into(),
            field: "line_weight[0]".into(),
            reason: "priority must be 1..=4".into(),
        };
        assert_eq!(editor_message(&e), "Line weight 1: priority must be 1..=4");
        let e = FormatError::Invalid {
            id: "x".into(),
            field: "text.tab_size".into(),
            reason: "1..=32".into(),
        };
        assert_eq!(editor_message(&e), "text.tab_size: 1..=32");
    }

    #[test]
    fn sample_lines_read_as_a_document() {
        assert_eq!(sample_lines("a\r\nb\rc\n"), ["a", "b", "c"]);
        assert_eq!(sample_lines("a\n\nb"), ["a", "", "b"]);
        assert!(sample_lines("").is_empty());
    }
}
