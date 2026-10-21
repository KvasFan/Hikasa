//! Line keys of one side (§2.13): the identity fast path writes raw lines into per-chunk arenas (D-53); the
//! general path runs `hikasa_formats::tokenize_all` (parallel speculative chunks) with a callback that writes
//! the keys of §2.13.2 into the tokenizer's arenas. Line weights (TC-012) are computed with the same chunking,
//! only when the algorithm resolves to Standard (after interning, because `Auto` depends on the keys, D-36).

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_formats::{CompiledFormat, LineBytes, LineState, Token, Tokenizer, tokenize_all};

use crate::classify::{KeyShape, RuleSet};
use crate::compare::rows::LONG_LINE_BYTES;
use crate::document::{LineIdx, TextDocument};
use crate::util::{PhaseProgress, par_map};

/// Lines per chunk (the tokenizer's chunk size; also the cancellation interval of key building, §5.3).
pub(crate) const CHUNK_LINES: u32 = 8192;

/// Lines between two progress reports of key building.
const PROGRESS_LINES: u32 = 1024;

/// One chunk of identity keys: one byte arena plus one end offset per line.
#[derive(Debug, Clone, Default)]
struct ArenaChunk {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

/// Identity keys of every line, in per-chunk arenas (no per-line allocation).
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyArena {
    chunks: Vec<ArenaChunk>,
    len: u32,
}

impl KeyArena {
    fn get(&self, line: u32) -> &[u8] {
        if line >= self.len {
            return &[];
        }
        let Some(chunk) = self.chunks.get((line / CHUNK_LINES) as usize) else {
            return &[];
        };
        let i = (line % CHUNK_LINES) as usize;
        let start = if i == 0 {
            0
        } else {
            chunk.ends.get(i - 1).copied().unwrap_or(0) as usize
        };
        let end = chunk.ends.get(i).copied().unwrap_or(0) as usize;
        chunk.bytes.get(start..end).unwrap_or(&[])
    }
}

/// The keys of one side.
#[derive(Debug, Clone)]
pub(crate) enum KeyStore {
    /// Raw lines (D-53).
    Identity(KeyArena),
    /// Atom keys written by the `tokenize_all` callback.
    General(LineBytes),
}

impl Default for KeyStore {
    /// No lines.
    fn default() -> KeyStore {
        KeyStore::Identity(KeyArena::default())
    }
}

impl KeyStore {
    /// Number of lines.
    pub(crate) fn len(&self) -> u32 {
        match self {
            KeyStore::Identity(a) => a.len,
            KeyStore::General(b) => b.len(),
        }
    }

    /// Key of `line` (`&[]` out of range).
    #[inline]
    pub(crate) fn get(&self, line: u32) -> &[u8] {
        match self {
            KeyStore::Identity(a) => a.get(line),
            KeyStore::General(b) => b.get(line),
        }
    }
}

/// Keys and start states of one side.
#[derive(Debug, Clone, Default)]
pub(crate) struct SideKeys {
    /// Keys.
    pub(crate) store: KeyStore,
    /// Tokenizer start state of each line (general path with a multi-line grammar; else empty).
    pub(crate) start_states: Vec<LineState>,
    /// Regex runtime-limit hits (tokenizer and unimportant patterns).
    pub(crate) regex_errors: u64,
}

/// What key building needs besides the document.
#[derive(Clone, Copy)]
pub(crate) struct KeyJob<'a> {
    /// Effective format.
    pub(crate) format: &'a CompiledFormat,
    /// Compiled rules.
    pub(crate) set: &'a RuleSet,
    /// Identity or general keys.
    pub(crate) shape: KeyShape,
    /// The caller's pool.
    pub(crate) pool: Option<&'a rayon::ThreadPool>,
    /// Cancellation.
    pub(crate) cancel: &'a CancelToken,
    /// The `tokenizing` progress.
    pub(crate) progress: &'a PhaseProgress<'a>,
}

impl SideKeys {
    /// Builds the keys of `doc`. On cancellation the keys may be partial; the caller checks the token after
    /// this call and discards them.
    pub(crate) fn build(doc: &TextDocument, job: &KeyJob<'_>) -> Result<SideKeys, Cancelled> {
        let KeyJob {
            format,
            set,
            shape,
            pool,
            cancel,
            progress,
        } = *job;
        if shape.identity {
            return Ok(SideKeys {
                store: KeyStore::Identity(identity_keys(doc, set, pool, cancel, progress)?),
                start_states: Vec::new(),
                regex_errors: 0,
            });
        }
        let tokenized = tokenize_all(
            format.grammar(),
            doc,
            pool,
            cancel,
            |line_no: u32, text: &str, tokens: &[Token], out: &mut Vec<u8>| {
                let eol = doc.line_ending(LineIdx(line_no));
                // The pattern hits are counted below: the callback also runs for lines that the speculative
                // fix-up re-tokenises, so counting here would depend on the pool (T2).
                set.write_key(text, tokens, eol, out, cancel);
                if line_no % PROGRESS_LINES == PROGRESS_LINES - 1 {
                    progress.add(u64::from(PROGRESS_LINES));
                }
            },
        )?;
        let regex_errors = tokenized.regex_errors + pattern_errors(doc, set, pool, cancel)?;
        Ok(SideKeys {
            store: KeyStore::General(tokenized.bytes),
            start_states: tokenized.start_states,
            regex_errors,
        })
    }
}

impl SideKeys {
    /// True when every line's key equals the key the line gets from the tokenizer's `Normal` state: the keys
    /// are then a function of each line's text and ending alone, so raw-identical lines have equal keys. Always
    /// true on the identity path and for grammars without multi-line items (no start states are kept); otherwise
    /// the lines that start in another state are re-tokenised from `Normal` (the quick compare's step-5 guard,
    /// §2.19).
    pub(crate) fn context_free(
        &self,
        doc: &TextDocument,
        format: &CompiledFormat,
        set: &RuleSet,
        cancel: &CancelToken,
    ) -> Result<bool, Cancelled> {
        if self.start_states.is_empty() {
            return Ok(true);
        }
        let mut tokenizer = Tokenizer::new(format.grammar());
        let mut tokens = Vec::new();
        let mut key = Vec::new();
        let mut cursor = doc.cursor();
        for (line, &state) in self.start_states.iter().enumerate() {
            let line = line as u32;
            if line.is_multiple_of(CHUNK_LINES) {
                cancel.check()?;
            }
            if state == LineState::Normal {
                continue;
            }
            let text = cursor.line(line);
            tokenizer.tokenize_line_cancellable(
                text,
                line,
                LineState::Normal,
                &mut tokens,
                cancel,
            )?;
            key.clear();
            set.write_key(
                text,
                &tokens,
                doc.line_ending(LineIdx(line)),
                &mut key,
                cancel,
            );
            cancel.check()?;
            if key != self.store.get(line) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Identity keys (D-53) in parallel chunks: each chunk walks the rope once with a sequential cursor.
fn identity_keys(
    doc: &TextDocument,
    set: &RuleSet,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
    progress: &PhaseProgress<'_>,
) -> Result<KeyArena, Cancelled> {
    let n = doc.line_count();
    let n_chunks = n.div_ceil(CHUNK_LINES) as usize;
    let chunks = par_map(pool, n_chunks, |k| {
        cancel.check()?;
        let first = k as u32 * CHUNK_LINES;
        let last = (first + CHUNK_LINES).min(n);
        // Exact-enough reservation: the chunk's text plus two marker bytes per line when endings are compared
        // (artificial breaks add one byte each; they are rare and covered by the terminators they replace).
        let span = match (doc.content_range(first), doc.content_range(last - 1)) {
            (Some((s, _)), Some((_, e))) => e.saturating_sub(s),
            _ => 0,
        };
        let mut chunk = ArenaChunk {
            bytes: Vec::with_capacity(
                span + if set.compare_eol {
                    2 * (last - first) as usize
                } else {
                    0
                },
            ),
            ends: Vec::with_capacity((last - first) as usize),
        };
        let mut cursor = doc.cursor();
        for line in first..last {
            let eol = doc.line_ending(LineIdx(line));
            if doc.line_len(line) < LONG_LINE_BYTES {
                set.write_identity_key(cursor.line(line), eol, &mut chunk.bytes);
            } else {
                // A long line is copied straight from the rope, piece by piece, with cancellation checks.
                let mut since = 0usize;
                for piece in doc.line_chunks(line) {
                    chunk.bytes.extend_from_slice(piece.as_bytes());
                    since += piece.len();
                    if since >= 1 << 20 {
                        since = 0;
                        cancel.check()?;
                    }
                }
                set.push_identity_markers(eol, &mut chunk.bytes);
            }
            chunk.ends.push(chunk.bytes.len() as u32);
            if line % PROGRESS_LINES == PROGRESS_LINES - 1 {
                progress.add(u64::from(PROGRESS_LINES));
            }
        }
        Ok(chunk)
    })?;
    Ok(KeyArena { chunks, len: n })
}

/// Runtime-limit hits of the unimportant patterns over every line (only fancy-regex patterns can hit the
/// limit; none → no pass). A pattern's hits depend only on the line's text, so a separate pass counts every line
/// exactly once, whatever the pool. The matches are streamed (no per-match memory).
fn pattern_errors(
    doc: &TextDocument,
    set: &RuleSet,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
) -> Result<u64, Cancelled> {
    if !set.has_fancy_patterns() {
        return Ok(0);
    }
    let n = doc.line_count();
    let n_chunks = n.div_ceil(CHUNK_LINES) as usize;
    let parts = par_map(pool, n_chunks, |k| {
        cancel.check()?;
        let first = k as u32 * CHUNK_LINES;
        let last = (first + CHUNK_LINES).min(n);
        let mut cursor = doc.cursor();
        let mut errors = 0u64;
        for line in first..last {
            let mut pats = set.patterns(cursor.line(line), cancel);
            pats.finish();
            if pats.cancelled() {
                return Err(Cancelled);
            }
            errors += u64::from(pats.errors());
        }
        Ok(errors)
    })?;
    Ok(parts.into_iter().sum())
}

/// Line weights (TC-012, §2.10.7) of every line, with the weight regexes' runtime-limit hits. Empty when the
/// format has no weights.
pub(crate) fn line_weights(
    doc: &TextDocument,
    format: &CompiledFormat,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
) -> Result<(Vec<u8>, u64), Cancelled> {
    if !format.has_line_weights() {
        return Ok((Vec::new(), 0));
    }
    let n = doc.line_count();
    let n_chunks = n.div_ceil(CHUNK_LINES) as usize;
    let parts = par_map(pool, n_chunks, |k| {
        cancel.check()?;
        let first = k as u32 * CHUNK_LINES;
        let last = (first + CHUNK_LINES).min(n);
        let mut cursor = doc.cursor();
        let mut weights = Vec::with_capacity((last - first) as usize);
        let mut errors = 0u64;
        for line in first..last {
            let (w, e) = format.line_weight_with_errors(cursor.line(line));
            weights.push(w);
            errors += u64::from(e);
        }
        Ok((weights, errors))
    })?;
    let mut weights = Vec::with_capacity(n as usize);
    let mut errors = 0u64;
    for (w, e) in parts {
        weights.extend_from_slice(&w);
        errors += e;
    }
    Ok((weights, errors))
}
