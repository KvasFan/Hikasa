//! Building documents: [`TextDocument::from_bytes`] (pure), [`TextDocument::from_text`] and
//! [`load_document`] (the only I/O entry point of the crate), with the Sorted conversion (D-38) and the External
//! conversion's byte stage (D-39).

use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::progress::{Ctl, Phase, Progress, Unit};
use hikasa_core::spec::Spec;
use hikasa_formats::{
    CompiledFormat, Conversion, ConvertError, DetectedEncoding, EncodingSource, ProcessRunner,
    TextEncoding, detect, run_external, sniff,
};
use hikasa_vfs::{EntryKind, FollowLinks, StreamOptions, Vfs, VfsError, VfsOp};

use super::lines::{BUILD_STEP, LineTable, MAX_LINES, check_text_len};
use super::{DocOrigin, LoadOptions, MAX_DOCUMENT_BYTES, TextDocument};
use crate::error::TextError;
use crate::util::release_later;
use hikasa_core::side::Side;

/// Bytes read between two cancellation checks and progress reports (§5.3).
const READ_STEP: usize = 1 << 20;

impl TextDocument {
    /// Builds a document from raw bytes. Pure: binary sniff → Ctrl-Z → detect/decode → line split →
    /// Sorted conversion (when the format says so) → CPL wrapping. Never fails on content; it fails only on
    /// `max_bytes` (`TooLarge`), on more than `MAX_LINES` lines (`TooManyLines`, D-43) or on an unknown
    /// encoding label.
    ///
    /// Valid UTF-8 content is not copied before the rope is built (the rope is the only copy). Decoded text over
    /// 4 GiB − 1 bytes (possible when a legacy encoding expands) is `TooLarge` as well, because the line table
    /// stores `u32` offsets. The External conversion is a byte stage run by [`load_document`] before this
    /// function; the binary verdict is taken from `bytes` as given. The same chunked pipeline runs in
    /// [`load_document`] with cancellation checks every 1 MiB (§5.3).
    pub fn from_bytes(
        bytes: &[u8],
        format: &CompiledFormat,
        opts: &LoadOptions,
        origin: DocOrigin,
    ) -> Result<TextDocument, TextError> {
        build(
            bytes,
            format,
            opts,
            origin,
            false,
            &CancelToken::never(),
            None,
        )
    }

    /// UTF-8 text with no detection (tests and stdin pipelines that are already text). Lines are split at CR, LF
    /// and CRLF; no characters-per-line limit applies. The binary verdict is sniffed from the text's bytes.
    ///
    /// Total (T9): text beyond the D-43 limits (more than 4 GiB − 1 bytes or [`MAX_LINES`] lines), which
    /// [`TextDocument::from_bytes`] rejects, is cut at the limit; [`TextDocument::truncated`] then returns true
    /// and a comparison of the document carries a `lossy_decode` warning saying so.
    pub fn from_text(text: &str) -> TextDocument {
        let spec = "<memory>";
        let never = CancelToken::never();
        let mut text = text;
        let mut truncated = false;
        let (lines, eol) = loop {
            match LineTable::build(text.as_bytes(), None, spec, &never) {
                Ok(built) => break built,
                Err(_) => {
                    text = cut_to_limits(text);
                    truncated = true;
                }
            }
        };
        TextDocument {
            rope: ropey::Rope::from_str(text),
            lines,
            encoding: DetectedEncoding {
                encoding: TextEncoding::UTF_8,
                bom_len: 0,
                source: EncodingSource::Utf8Valid,
            },
            malformed: 0,
            eol,
            binary: sniff(text.as_bytes()),
            converted: false,
            truncated,
            origin: DocOrigin {
                spec: None,
                mtime: None,
                size: text.len() as u64,
            },
        }
    }
}

/// The longest prefix of `text` within the D-43 limits (strictly shorter than `text` when it exceeds them).
fn cut_to_limits(text: &str) -> &str {
    let mut end = text.len().min(u32::MAX as usize);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let bytes = &text.as_bytes()[..end];
    // Cut after the MAX_LINES-th terminator (CRLF counts once).
    let mut lines = 0u64;
    let mut iter = memchr::memchr2_iter(b'\n', b'\r', bytes).peekable();
    while let Some(p) = iter.next() {
        let next = if bytes[p] == b'\r' && bytes.get(p + 1) == Some(&b'\n') {
            iter.next();
            p + 2
        } else {
            p + 1
        };
        lines += 1;
        if lines == u64::from(MAX_LINES) {
            end = next;
            break;
        }
    }
    if end == text.len() {
        // Defensive: always make progress.
        end = end.saturating_sub(1);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
    }
    &text[..end]
}

/// Decoded text: borrowed from the source when it is valid UTF-8 (no copy before the rope), else owned.
enum Text<'a> {
    Borrowed(&'a [u8]),
    Owned(String),
}

impl Text<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Text::Borrowed(b) => b,
            Text::Owned(s) => s.as_bytes(),
        }
    }
}

/// Content from this size on is given to the encoding detector on a helper thread when the caller owns it
/// (`load_document`): `hikasa_formats::detect` scans the whole content in one call (ASCII and UTF-8 validity,
/// D-42), which takes tens of milliseconds per 100 MiB, so the caller polls its token every 10 ms instead of
/// waiting (§5.3, as D-54 does for imara-diff). A cancelled detection is abandoned; its helper ends after its
/// one linear scan.
const HELPER_DETECT_BYTES: usize = 8 << 20;

/// [`TextDocument::from_bytes`] with the `converted` flag of an External conversion that already ran, checking
/// `cancel` between the stages and every 1 MiB inside the long ones (UTF-8 validation, decoding, the Sorted
/// conversion, the line split and the rope build; §5.3). `shared`, when given, owns `bytes` (`bytes` is all of
/// it), so a large detection can run on a helper thread.
fn build(
    bytes: &[u8],
    format: &CompiledFormat,
    opts: &LoadOptions,
    origin: DocOrigin,
    externally_converted: bool,
    cancel: &CancelToken,
    shared: Option<&Arc<Vec<u8>>>,
) -> Result<TextDocument, TextError> {
    let spec = origin.display();
    let limit = opts.effective_max_bytes();
    if bytes.len() as u64 > limit {
        return Err(TextError::TooLarge { spec, limit });
    }
    cancel.check()?;
    let binary = sniff(bytes);
    let content = ctrl_z_cut(bytes, format, opts, cancel, shared)?;
    cancel.check()?;
    let encoding = detect_prefix(content, format, opts, cancel, shared)?;
    cancel.check()?;
    let body = content.get(usize::from(encoding.bom_len)..).unwrap_or(&[]);
    // The &str pieces of valid UTF-8 (≤ 1 MiB each) are kept for the rope: validated once, copied once.
    let mut pieces: Option<Vec<&str>> = None;
    let (mut text, malformed): (Text<'_>, u64) = match encoding.encoding {
        TextEncoding::Whatwg(e) if e == encoding_rs::UTF_8 => match utf8_pieces(body, cancel)? {
            Some(p) => {
                pieces = Some(p);
                (Text::Borrowed(body), 0)
            }
            None => {
                let (s, m) = decode_whatwg(body, e, cancel)?;
                (Text::Owned(s), m)
            }
        },
        TextEncoding::Whatwg(e) => {
            let (s, m) = decode_whatwg(body, e, cancel)?;
            (Text::Owned(s), m)
        }
        TextEncoding::Utf32Le => {
            let (s, m) = decode_utf32(body, false, cancel)?;
            (Text::Owned(s), m)
        }
        TextEncoding::Utf32Be => {
            let (s, m) = decode_utf32(body, true, cancel)?;
            (Text::Owned(s), m)
        }
    };
    let mut converted = externally_converted;
    let built = (|| -> Result<_, TextError> {
        if matches!(format.conversion(), Conversion::Sorted) {
            converted = true;
            let sorted = sort_lines(text.bytes(), cancel)?;
            // The unsorted text is no longer needed.
            match std::mem::replace(&mut text, Text::Owned(sorted)) {
                Text::Owned(old) if cancel.is_cancelled() => release_later(old),
                old => drop(old),
            }
            pieces = None;
        }
        check_text_len(text.bytes().len(), &spec)?;
        let cpl = opts
            .chars_per_line_limit
            .or(format.text().chars_per_line_limit);
        let (lines, eol) = LineTable::build(text.bytes(), cpl, &spec, cancel)?;
        let lines = crate::util::ReleaseOnCancel::new(lines, cancel);
        let rope = match (&text, pieces.take()) {
            (_, Some(pieces)) => rope_from_pieces(pieces.into_iter(), cancel)?,
            (Text::Owned(s), None) => rope_from_pieces(str_pieces(s), cancel)?,
            (Text::Borrowed(b), None) => {
                // Unreachable: borrowed text always comes with its validated pieces.
                let pieces = utf8_pieces(b, cancel)?.unwrap_or_default();
                rope_from_pieces(pieces.into_iter(), cancel)?
            }
        };
        let mut lines = lines;
        Ok((std::mem::take(&mut lines.value), eol, rope))
    })();
    // On a cancellation the decoded text (hundreds of MiB for a large legacy-encoded file) is freed off this
    // thread.
    match text {
        Text::Owned(s) if built.is_err() => release_later(s),
        other => drop(other),
    }
    let (lines, eol, rope) = built?;
    if cancel.is_cancelled() {
        release_later((lines, rope));
        return Err(TextError::Cancelled);
    }
    Ok(TextDocument {
        rope,
        lines,
        encoding,
        malformed,
        eol,
        binary,
        converted,
        truncated: false,
        origin,
    })
}

/// A UTF-8 continuation byte.
#[inline]
fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// Validates `bytes` as UTF-8 in pieces of about 1 MiB (split before a char's first byte), checking
/// `cancel` between pieces; the pieces as `&str`, or `None` when the bytes are not valid UTF-8.
fn utf8_pieces<'a>(
    bytes: &'a [u8],
    cancel: &CancelToken,
) -> Result<Option<Vec<&'a str>>, Cancelled> {
    let mut pieces = Vec::with_capacity(bytes.len() / BUILD_STEP + 1);
    let mut pos = 0usize;
    while pos < bytes.len() {
        cancel.check()?;
        let mut end = (pos + BUILD_STEP).min(bytes.len());
        while end < bytes.len() && end > pos && is_continuation(bytes[end]) {
            end -= 1;
        }
        if end == pos {
            end = (pos + BUILD_STEP).min(bytes.len());
        }
        match std::str::from_utf8(&bytes[pos..end]) {
            Ok(s) => pieces.push(s),
            Err(_) => return Ok(None),
        }
        pos = end;
    }
    Ok(Some(pieces))
}

/// `text` in pieces of about 1 MiB, split at char boundaries.
fn str_pieces(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut cut = rest.len().min(BUILD_STEP);
        while !rest.is_char_boundary(cut) {
            cut += 1;
        }
        let (piece, tail) = rest.split_at(cut);
        rest = tail;
        Some(piece)
    })
}

/// Builds the rope piece by piece, checking `cancel` between pieces.
fn rope_from_pieces<'a>(
    pieces: impl Iterator<Item = &'a str>,
    cancel: &CancelToken,
) -> Result<ropey::Rope, Cancelled> {
    let mut builder = ropey::RopeBuilder::new();
    for piece in pieces {
        if cancel.is_cancelled() {
            release_later(builder);
            return Err(Cancelled);
        }
        builder.append(piece);
    }
    Ok(builder.finish())
}

/// `hikasa_formats::decode` of a WHATWG encoding, streamed in 1 MiB input pieces through one stateful
/// `encoding_rs` decoder (a sequence split between pieces is completed by the next one), checking `cancel`
/// between pieces. Malformed sequences become U+FFFD and are counted exactly as by the one-shot decode.
fn decode_whatwg(
    body: &[u8],
    encoding: &'static encoding_rs::Encoding,
    cancel: &CancelToken,
) -> Result<(String, u64), Cancelled> {
    decode_whatwg_in(body, encoding, cancel, BUILD_STEP)
}

/// [`decode_whatwg`] with an explicit piece size (tests use tiny pieces).
fn decode_whatwg_in(
    body: &[u8],
    encoding: &'static encoding_rs::Encoding,
    cancel: &CancelToken,
    step: usize,
) -> Result<(String, u64), Cancelled> {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let step = step.max(1);
    // Each piece is decoded into a small reused buffer, then appended. encoding_rs initialises one byte of every
    // page of a String's spare capacity on every call, so decoding straight into a worst-case reservation of
    // the whole output (up to 3 × the input) would fault all of it in at the first call: hundreds of MiB at
    // once, an uninterruptible stall under memory pressure.
    let piece_capacity = decoder
        .max_utf8_buffer_length_without_replacement(step)
        .unwrap_or(step.saturating_mul(3))
        .max(16);
    let mut piece_out = String::with_capacity(piece_capacity);
    let mut out = String::with_capacity(body.len());
    let mut malformed = 0u64;
    let mut pos = 0usize;
    loop {
        if cancel.is_cancelled() {
            release_later(out);
            return Err(Cancelled);
        }
        let end = (pos + step).min(body.len());
        let last = end == body.len();
        let mut piece = &body[pos..end];
        loop {
            let (result, read) =
                decoder.decode_to_string_without_replacement(piece, &mut piece_out, last);
            piece = &piece[read..];
            match result {
                encoding_rs::DecoderResult::InputEmpty => break,
                encoding_rs::DecoderResult::OutputFull => {
                    if piece_out.is_empty() {
                        let more = decoder
                            .max_utf8_buffer_length_without_replacement(piece.len())
                            .unwrap_or(4096)
                            .max(16);
                        piece_out.reserve(more);
                    } else {
                        out.push_str(&piece_out);
                        piece_out.clear();
                    }
                }
                encoding_rs::DecoderResult::Malformed(_, _) => {
                    malformed += 1;
                    piece_out.push('\u{FFFD}');
                }
            }
        }
        out.push_str(&piece_out);
        piece_out.clear();
        pos = end;
        if last {
            break;
        }
    }
    Ok((out, malformed))
}

/// `hikasa_formats::decode` of UTF-32 (Hikasa's decoder: 4-byte units; an invalid scalar value or 1–3
/// trailing bytes → one U+FFFD each), in 1 MiB pieces with cancellation checks.
fn decode_utf32(
    body: &[u8],
    big_endian: bool,
    cancel: &CancelToken,
) -> Result<(String, u64), Cancelled> {
    let mut out = String::with_capacity(body.len() / 4 + 4);
    let mut malformed = 0u64;
    let (units, rest) = body.as_chunks::<4>();
    for block in units.chunks(BUILD_STEP / 4) {
        if cancel.is_cancelled() {
            release_later(out);
            return Err(Cancelled);
        }
        for &bytes in block {
            let v = if big_endian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            };
            match char::from_u32(v) {
                Some(c) => out.push(c),
                None => {
                    malformed += 1;
                    out.push('\u{FFFD}');
                }
            }
        }
    }
    if !rest.is_empty() {
        malformed += 1;
        out.push('\u{FFFD}');
    }
    Ok((out, malformed))
}

/// The Sorted conversion (D-38), identical to `hikasa_formats::sort_lines` and cancellable: lines split at LF,
/// CRLF and CR; stably sorted by code point (UTF-8 byte order); joined with the dominant ending (ties
/// LF > CRLF > CR); a final ending only if the text had one. The stable sort is a bottom-up merge sort whose
/// initial runs hold about 1 MiB of text or 4,096 lines, so every comparison between two cancellation checks
/// touches a bounded number of bytes.
fn sort_lines(text: &[u8], cancel: &CancelToken) -> Result<String, Cancelled> {
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut start = 0usize;
    let mut at = 0usize;
    let mut next_check = BUILD_STEP;
    while let Some(off) = memchr::memchr2(b'\n', b'\r', &text[at..]) {
        let p = at + off;
        lines.push((start, p));
        if text[p] == b'\n' {
            lf += 1;
            at = p + 1;
        } else if text.get(p + 1) == Some(&b'\n') {
            crlf += 1;
            at = p + 2;
        } else {
            cr += 1;
            at = p + 1;
        }
        start = at;
        if at >= next_check {
            cancel.check()?;
            next_check = at + BUILD_STEP;
        }
    }
    let ends_with_eol = start == text.len() && !text.is_empty();
    if start < text.len() {
        lines.push((start, text.len()));
    }
    let eol: &[u8] = if lf >= crlf && lf >= cr {
        b"\n"
    } else if crlf >= cr {
        b"\r\n"
    } else {
        b"\r"
    };
    let line = |&(s, e): &(usize, usize)| &text[s..e];
    // Initial runs, sorted stably.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut run_start = 0usize;
    let mut run_bytes = 0usize;
    for (k, l) in lines.iter().enumerate() {
        run_bytes += l.1 - l.0;
        if run_bytes >= BUILD_STEP || k + 1 - run_start >= 4096 || k + 1 == lines.len() {
            runs.push((run_start, k + 1));
            run_start = k + 1;
            run_bytes = 0;
        }
    }
    for &(s, e) in &runs {
        cancel.check()?;
        lines[s..e].sort_by(|a, b| line(a).cmp(line(b)));
    }
    // Bottom-up stable merges (left run first on ties), checking `cancel` every 1 MiB of compared bytes.
    let mut buffer: Vec<(usize, usize)> = Vec::with_capacity(lines.len());
    let mut budget = 0usize;
    while runs.len() > 1 {
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(runs.len().div_ceil(2));
        for pair in runs.chunks(2) {
            let &[(s, mid), (_, e)] = pair else {
                merged.push(pair[0]);
                continue;
            };
            buffer.clear();
            let (mut i, mut j) = (s, mid);
            while i < mid && j < e {
                let (a, b) = (line(&lines[i]), line(&lines[j]));
                budget += a.len().min(b.len()) + 1;
                if budget >= BUILD_STEP {
                    budget = 0;
                    cancel.check()?;
                }
                if b < a {
                    buffer.push(lines[j]);
                    j += 1;
                } else {
                    buffer.push(lines[i]);
                    i += 1;
                }
            }
            buffer.extend_from_slice(&lines[i..mid]);
            buffer.extend_from_slice(&lines[j..e]);
            lines[s..e].copy_from_slice(&buffer);
            merged.push((s, e));
        }
        runs = merged;
    }
    // Each line is valid UTF-8 (the text is, and lines end before ASCII terminators), so it is validated and
    // copied once, with cancellation checks every 1 MiB of output.
    let eol = std::str::from_utf8(eol).unwrap_or("\n");
    let mut out = String::with_capacity(text.len());
    let mut next_check = BUILD_STEP;
    for (k, l) in lines.iter().enumerate() {
        if out.len() >= next_check {
            cancel.check()?;
            next_check = out.len() + BUILD_STEP;
        }
        match std::str::from_utf8(line(l)) {
            Ok(s) => out.push_str(s),
            Err(_) => out.push_str(&String::from_utf8_lossy(line(l))),
        }
        if k + 1 < lines.len() || ends_with_eol {
            out.push_str(eol);
        }
    }
    Ok(out)
}

/// `hikasa_formats::detect` of `content`, a prefix of `shared` when that is given: on a helper thread polled
/// every 10 ms when the content is large (see [`HELPER_DETECT_BYTES`]), else inline.
fn detect_prefix(
    content: &[u8],
    format: &CompiledFormat,
    opts: &LoadOptions,
    cancel: &CancelToken,
    shared: Option<&Arc<Vec<u8>>>,
) -> Result<DetectedEncoding, TextError> {
    // `content` is a prefix of the shared bytes (the Ctrl-Z cut keeps the start): same start, not longer.
    let Some(shared) = shared.filter(|s| {
        content.len() >= HELPER_DETECT_BYTES
            && content.len() <= s.len()
            && std::ptr::eq(content.as_ptr(), s.as_ptr())
    }) else {
        return Ok(detect(content, format.encoding(), &opts.decode)?);
    };
    let data = Arc::clone(shared);
    let len = content.len();
    let setting = format.encoding().clone();
    let decode = opts.decode;
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hikasa-text-detect".into())
        .spawn(move || {
            let result = detect(data.get(..len).unwrap_or(&[]), &setting, &decode);
            // Release the bytes before answering, so the caller can take them back.
            drop(data);
            let _ = tx.send(result);
        });
    let Ok(helper) = spawned else {
        return Ok(detect(content, format.encoding(), &opts.decode)?);
    };
    loop {
        match rx.recv_timeout(DETECT_POLL) {
            Ok(result) => {
                let _ = helper.join();
                return Ok(result?);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.is_cancelled() {
                    return Err(TextError::Cancelled);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = helper.join();
                return Ok(detect(content, format.encoding(), &opts.decode)?);
            }
        }
    }
}

/// Poll interval of a detection on a helper thread (§5.3).
const DETECT_POLL: Duration = Duration::from_millis(10);

/// The Ctrl-Z rule (TextSettings::ctrl_z_eof): with the flag set, content up to the first 0x1A when the
/// content's encoding is ASCII-compatible (detected on the whole content first).
fn ctrl_z_cut<'b>(
    bytes: &'b [u8],
    format: &CompiledFormat,
    opts: &LoadOptions,
    cancel: &CancelToken,
    shared: Option<&Arc<Vec<u8>>>,
) -> Result<&'b [u8], TextError> {
    if !format.text().ctrl_z_eof {
        return Ok(bytes);
    }
    let mut found = None;
    for (k, block) in bytes.chunks(BUILD_STEP).enumerate() {
        if k > 0 {
            cancel.check()?;
        }
        if let Some(off) = memchr::memchr(0x1A, block) {
            found = Some(k * BUILD_STEP + off);
            break;
        }
    }
    let Some(at) = found else {
        return Ok(bytes);
    };
    let whole = detect_prefix(bytes, format, opts, cancel, shared)?;
    if whole.encoding.is_ascii_compatible() {
        Ok(&bytes[..at])
    } else {
        Ok(bytes)
    }
}

/// Loads `spec` through `vfs`: stat, size check, read (cancellation checked per 1 MiB), optional External
/// conversion (byte stage, run in `temp_dir` with `runner`), then `from_bytes`. The only I/O entry point of the
/// crate. Returns the raw bytes too, so the caller can compute `bytes_identical` and then drop them.
///
/// Errors: VFS failures are `TextError::Load` with `side: Side::Left` (the function does not know the side;
/// callers apply [`TextError::with_side`]); a source over `opts.max_bytes` is `TooLarge` before anything is read
/// (a reported size of at most 4096 bytes is not trusted for that, as in hikasa-vfs, but the read stops at the
/// limit); a format whose conversion is External needs `external`, otherwise the load fails with
/// `TextError::Convert` (`unsupported`), never silently unconverted. Progress: `reading` (bytes) per 1 MiB and
/// `decoding` (bytes) at its start and end (§5.4). Cancellation is checked every 1 MiB while reading and while
/// the document is built (UTF-8 validation, decoding, Sorted conversion, line split, rope), and once more before
/// returning: a cancelled load is `Err(TextError::Cancelled)`, never `Ok` (§5.3).
pub fn load_document(
    vfs: &dyn Vfs,
    spec: &Spec,
    format: &CompiledFormat,
    opts: &LoadOptions,
    external: Option<(&Path, &dyn ProcessRunner)>,
    ctl: &Ctl<'_>,
) -> Result<(TextDocument, Vec<u8>), TextError> {
    ctl.check()?;
    let canonical = spec.canonical();
    let load_error = |source: VfsError| -> TextError {
        if source.is_cancelled() {
            TextError::Cancelled
        } else {
            TextError::Load {
                side: Side::Left,
                source,
            }
        }
    };
    let entry = vfs.stat(spec, FollowLinks::Yes).map_err(load_error)?;
    if entry.kind == EntryKind::Dir {
        return Err(load_error(VfsError::IsADirectory {
            spec: canonical.clone(),
        }));
    }
    let limit = opts.effective_max_bytes();
    if entry.size > limit && entry.size > StreamOptions::UNRELIABLE_SIZE_MAX {
        return Err(TextError::TooLarge {
            spec: canonical,
            limit,
        });
    }
    let mut reader = vfs.open_read(spec).map_err(load_error)?;
    let mut raw = read_all(&mut reader, entry.size, limit, ctl).map_err(|e| match e {
        ReadFail::Cancelled => TextError::Cancelled,
        ReadFail::TooLarge => TextError::TooLarge {
            spec: canonical.clone(),
            limit,
        },
        ReadFail::Io(error) => load_error(VfsError::from_io(canonical.clone(), VfsOp::Read, error)),
    })?;
    drop(reader);
    let origin = DocOrigin {
        spec: Some(spec.clone()),
        mtime: entry.mtime,
        size: raw.len() as u64,
    };
    let total = raw.len() as u64;
    ctl.progress_now(Progress {
        phase: Phase::Decoding,
        done: 0,
        total: Some(total),
        unit: Unit::Bytes,
        current: Some(canonical.clone()),
    });
    let doc = match format.conversion() {
        Conversion::External(conv) => {
            let Some((temp_dir, runner)) = external else {
                return Err(external_unsupported(&canonical, format));
            };
            let converted = run_external(conv, &raw, &canonical, temp_dir, runner, ctl.cancel)
                .map_err(|e| match e {
                    ConvertError::Cancelled => TextError::Cancelled,
                    other => TextError::Convert(other),
                })?;
            let converted = Arc::new(converted);
            build(
                &converted,
                format,
                opts,
                origin,
                true,
                ctl.cancel,
                Some(&converted),
            )?
        }
        Conversion::None | Conversion::Sorted => {
            let shared = Arc::new(raw);
            // On an error (a cancellation included) the bytes are dropped here, or by an abandoned detection
            // helper when it ends; never copied.
            let doc = build(
                &shared,
                format,
                opts,
                origin,
                false,
                ctl.cancel,
                Some(&shared),
            )?;
            // A finished helper released its reference before answering, so this never copies in practice.
            raw = Arc::try_unwrap(shared).unwrap_or_else(|shared| shared.as_ref().clone());
            doc
        }
    };
    // A cancellation that lands while the document is built is never answered with Ok (§5.3).
    ctl.check()?;
    ctl.progress_now(Progress {
        phase: Phase::Decoding,
        done: total,
        total: Some(total),
        unit: Unit::Bytes,
        current: Some(canonical),
    });
    Ok((doc, raw))
}

/// The error of a format whose conversion is External when no conversion runner is given (shared by
/// [`load_document`] and the quick compare, so that every path fails alike, TC-016).
pub(crate) fn external_unsupported(name: &str, format: &CompiledFormat) -> TextError {
    TextError::Convert(ConvertError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "{name}: format {} needs an external conversion program, which is not enabled here",
            format.name()
        ),
    )))
}

/// [`TextDocument::from_bytes`] for the quick compare: the `converted` flag of an External conversion that
/// already ran, and cancellation checks every 1 MiB.
pub(crate) fn document_from_bytes(
    bytes: &[u8],
    format: &CompiledFormat,
    opts: &LoadOptions,
    origin: DocOrigin,
    externally_converted: bool,
    cancel: &CancelToken,
) -> Result<TextDocument, TextError> {
    build(
        bytes,
        format,
        opts,
        origin,
        externally_converted,
        cancel,
        None,
    )
}

/// Why [`read_all`] stopped.
enum ReadFail {
    Cancelled,
    TooLarge,
    Io(io::Error),
}

/// Reads everything into an exact-size buffer (when `expected` is right), checking cancellation between the
/// partial reads and reporting `reading` progress every 1 MiB; more than `limit` bytes is `TooLarge`.
fn read_all(
    r: &mut dyn Read,
    expected: u64,
    limit: u64,
    ctl: &Ctl<'_>,
) -> Result<Vec<u8>, ReadFail> {
    let cancel: &CancelToken = ctl.cancel;
    let mut target = usize::try_from(expected.min(limit).min(MAX_DOCUMENT_BYTES)).unwrap_or(0);
    let mut buf: Vec<u8> = Vec::new();
    buf.try_reserve_exact(target)
        .map_err(|_| ReadFail::Io(io::Error::from(io::ErrorKind::OutOfMemory)))?;
    let total = (expected > 0).then_some(expected);
    loop {
        cancel.check().map_err(|_| ReadFail::Cancelled)?;
        let at_end = if buf.len() < target {
            let old = buf.len();
            let want = READ_STEP.min(target - old);
            buf.resize(old + want, 0);
            let n = fill(r, &mut buf[old..], cancel)?;
            buf.truncate(old + n);
            n < want
        } else {
            // At the expected size (or the size is unknown): probe for more.
            let mut probe = [0u8; 8192];
            let n = fill(r, &mut probe, cancel)?;
            buf.extend_from_slice(&probe[..n]);
            target = buf.len().saturating_add(READ_STEP);
            n < probe.len()
        };
        if buf.len() as u64 > limit {
            return Err(ReadFail::TooLarge);
        }
        let done = buf.len() as u64;
        ctl.progress(Phase::Reading, || Progress {
            phase: Phase::Reading,
            done,
            total: total.map(|t| t.max(done)),
            unit: Unit::Bytes,
            current: None,
        });
        if at_end {
            break;
        }
    }
    let done = buf.len() as u64;
    ctl.progress_now(Progress {
        phase: Phase::Reading,
        done,
        total: Some(done),
        unit: Unit::Bytes,
        current: None,
    });
    Ok(buf)
}

/// Fills `buf` from `r` until it is full or the reader ends (fewer bytes then), checking `cancel` between the
/// partial reads.
fn fill(r: &mut dyn Read, buf: &mut [u8], cancel: &CancelToken) -> Result<usize, ReadFail> {
    let mut n = 0;
    while n < buf.len() {
        cancel.check().map_err(|_| ReadFail::Cancelled)?;
        let got = read_some(r, &mut buf[n..])?;
        if got == 0 {
            break;
        }
        n += got;
    }
    Ok(n)
}

/// One `read` call, retried on `Interrupted`.
fn read_some(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize, ReadFail> {
    loop {
        match r.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if hikasa_core::io_error_kind(&e) == hikasa_core::ErrorKind::Cancelled => {
                return Err(ReadFail::Cancelled);
            }
            Err(e) => return Err(ReadFail::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{EolStyle, LineEnding, LineIdx};
    use hikasa_formats::{FileFormat, FormatId, FormatKind, RegexLimits};
    use std::sync::Arc;

    fn plain() -> CompiledFormat {
        CompiledFormat::everything_else()
    }

    fn with_text(f: impl FnOnce(&mut FileFormat)) -> CompiledFormat {
        let mut format = FileFormat::new(FormatId::new("t").unwrap(), "T", FormatKind::Text);
        f(&mut format);
        CompiledFormat::compile(Arc::new(format), &RegexLimits::default()).unwrap()
    }

    fn lines(doc: &TextDocument) -> Vec<String> {
        (0..doc.line_count())
            .map(|i| doc.line(LineIdx(i)).into_owned())
            .collect()
    }

    #[test]
    fn utf8_with_bom_and_legacy_encodings() {
        let doc = TextDocument::from_bytes(
            b"\xEF\xBB\xBFab\ncd",
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["ab", "cd"]);
        assert_eq!(doc.encoding().bom_len, 3);
        assert_eq!(doc.encoding().source, EncodingSource::Bom);
        let doc = TextDocument::from_bytes(
            b"a\x00b\x00\n\x00",
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["ab"]);
        assert_eq!(doc.encoding().encoding, TextEncoding::UTF_16LE);
        // One stray byte in UTF-8 text: lossy UTF-8 (D-42), counted.
        let mut bytes = "привет мир, ".repeat(20).into_bytes();
        bytes.push(0xFF);
        let doc = TextDocument::from_bytes(
            &bytes,
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(doc.encoding().source, EncodingSource::Utf8Lossy);
        assert_eq!(doc.malformed(), 1);
        assert!(doc.line(LineIdx(0)).ends_with('\u{FFFD}'));
    }

    #[test]
    fn limits() {
        let opts = LoadOptions {
            max_bytes: 3,
            ..LoadOptions::default()
        };
        let err =
            TextDocument::from_bytes(b"abcd", &plain(), &opts, DocOrigin::default()).unwrap_err();
        assert!(matches!(err, TextError::TooLarge { limit: 3, .. }), "{err}");
        let opts = LoadOptions {
            max_bytes: u64::MAX,
            ..LoadOptions::default()
        };
        assert_eq!(opts.effective_max_bytes(), MAX_DOCUMENT_BYTES);
    }

    #[test]
    fn the_sorted_conversion() {
        let sorted = with_text(|f| f.conversion = Conversion::Sorted);
        let doc = TextDocument::from_bytes(
            b"b\r\nc\r\na\n",
            &sorted,
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert!(doc.converted());
        assert_eq!(lines(&doc), ["a", "b", "c"]);
        // The dominant ending (CRLF) for every line; a final EOL only if the original had one (D-38).
        assert_eq!(doc.eol().style(), EolStyle::CrLf);
        let doc = TextDocument::from_bytes(
            b"z\ny",
            &sorted,
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["y", "z"]);
        assert_eq!(doc.line_ending(LineIdx(1)), LineEnding::None);
        let plain_doc = TextDocument::from_bytes(
            b"b\na\n",
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert!(!plain_doc.converted());
    }

    #[test]
    fn characters_per_line_limit_from_the_format_and_the_options() {
        let limited = with_text(|f| f.text.chars_per_line_limit = Some(2));
        let doc = TextDocument::from_bytes(
            b"abcde\n",
            &limited,
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["ab", "cd", "e"]);
        assert_eq!(doc.line_ending(LineIdx(0)), LineEnding::Artificial);
        assert_eq!(doc.physical_line(LineIdx(2)), 1);
        let opts = LoadOptions {
            chars_per_line_limit: Some(0),
            ..LoadOptions::default()
        };
        let doc =
            TextDocument::from_bytes(b"abcde\n", &limited, &opts, DocOrigin::default()).unwrap();
        assert_eq!(lines(&doc), ["abcde"]);
    }

    #[test]
    fn ctrl_z_ends_ascii_compatible_text() {
        let ctrl_z = with_text(|f| f.text.ctrl_z_eof = true);
        let doc = TextDocument::from_bytes(
            b"ab\ncd\x1Agarbage\n",
            &ctrl_z,
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["ab", "cd"]);
        let doc = TextDocument::from_bytes(
            b"ab\ncd\x1Agarbage\n",
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert_eq!(lines(&doc), ["ab", "cd\x1Agarbage"]);
    }

    #[test]
    fn binary_content_is_still_decoded() {
        let doc = TextDocument::from_bytes(
            b"\x89PNG\r\n\x1a\nrest",
            &plain(),
            &LoadOptions::default(),
            DocOrigin::default(),
        )
        .unwrap();
        assert!(doc.binary().is_binary);
        assert!(doc.line_count() > 0);
    }

    /// Seeded xorshift64* (no `rand` crate).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    /// The chunked decoder gives exactly `hikasa_formats::decode` (text and malformed count) for every encoding
    /// and piece size, including sequences split between pieces and malformed input.
    #[test]
    fn chunked_decoding_equals_the_one_shot_decode() {
        let encodings = [
            encoding_rs::UTF_8,
            encoding_rs::UTF_16LE,
            encoding_rs::UTF_16BE,
            encoding_rs::WINDOWS_1252,
            encoding_rs::SHIFT_JIS,
            encoding_rs::GB18030,
            encoding_rs::EUC_KR,
            encoding_rs::ISO_2022_JP,
            encoding_rs::BIG5,
        ];
        let mut rng = Rng(7);
        let never = CancelToken::never();
        for round in 0..300 {
            let len = (rng.next() % 64) as usize;
            let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            if round % 3 == 0 {
                // Mostly valid text with structure: UTF-8 and escape sequences.
                bytes = "aé€😀\x1b$B\x30\x21\x1b(B\r\n"
                    .repeat(1 + round % 4)
                    .into_bytes();
                let cut = (rng.next() as usize) % bytes.len();
                bytes.truncate(cut);
            }
            for &e in &encodings {
                let enc = DetectedEncoding {
                    encoding: TextEncoding::Whatwg(e),
                    bom_len: 0,
                    source: EncodingSource::Override,
                };
                let reference = hikasa_formats::decode(&bytes, &enc);
                for step in [1usize, 2, 3, 5, 7, 64] {
                    let (text, malformed) = decode_whatwg_in(&bytes, e, &never, step).unwrap();
                    assert_eq!(
                        (text.as_str(), malformed),
                        (reference.text.as_str(), reference.malformed),
                        "{} step {step} bytes {bytes:?}",
                        e.name()
                    );
                }
            }
            for big_endian in [false, true] {
                let enc = DetectedEncoding {
                    encoding: if big_endian {
                        TextEncoding::Utf32Be
                    } else {
                        TextEncoding::Utf32Le
                    },
                    bom_len: 0,
                    source: EncodingSource::Override,
                };
                let reference = hikasa_formats::decode(&bytes, &enc);
                assert_eq!(
                    decode_utf32(&bytes, big_endian, &never).unwrap(),
                    (reference.text, reference.malformed)
                );
            }
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(decode_whatwg(b"abc", encoding_rs::WINDOWS_1252, &cancel).is_err());
    }

    /// The UTF-8 pieces are split before a char's first byte and validate exactly when the whole text does.
    #[test]
    fn utf8_pieces_split_at_char_boundaries() {
        let never = CancelToken::never();
        let text = "aé€😀".repeat(BUILD_STEP / 5);
        let pieces = utf8_pieces(text.as_bytes(), &never).unwrap().unwrap();
        assert!(pieces.len() > 1);
        assert_eq!(pieces.concat(), text);
        let mut bad = text.clone().into_bytes();
        let at = BUILD_STEP + 1;
        bad[at] = 0xFF;
        assert_eq!(utf8_pieces(&bad, &never).unwrap(), None);
        assert_eq!(utf8_pieces(b"", &never).unwrap(), Some(Vec::new()));
        assert_eq!(str_pieces(&text).collect::<String>(), text);
    }

    /// The cancellable Sorted conversion equals `hikasa_formats::sort_lines`.
    #[test]
    fn sorted_conversion_equals_the_formats_sort() {
        let never = CancelToken::never();
        let mut rng = Rng(11);
        let alphabet = ["a", "b", "é", "\n", "\r\n", "\r", "ab", "", "z"];
        for _ in 0..500 {
            let n = (rng.next() % 30) as usize;
            let text: String = (0..n)
                .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
                .collect();
            assert_eq!(
                sort_lines(text.as_bytes(), &never).unwrap(),
                hikasa_formats::sort_lines(&text),
                "{text:?}"
            );
        }
        // Many lines: several runs and merge levels.
        let text: String = (0..20_000u64)
            .map(|k| format!("{}\n", (k * 7_919) % 1_000))
            .collect();
        assert_eq!(
            sort_lines(text.as_bytes(), &never).unwrap(),
            hikasa_formats::sort_lines(&text)
        );
        let cancel = CancelToken::new();
        cancel.cancel();
        let big = "x\n".repeat(BUILD_STEP);
        assert!(sort_lines(big.as_bytes(), &cancel).is_err());
    }

    #[test]
    fn from_text_cuts_only_beyond_the_limits() {
        assert_eq!(cut_to_limits("abc"), "ab");
        let doc = TextDocument::from_text("a\nb");
        assert_eq!(lines(&doc), ["a", "b"]);
        assert_eq!(doc.origin().display(), "<memory>");
        assert!(!doc.truncated());
    }
}
