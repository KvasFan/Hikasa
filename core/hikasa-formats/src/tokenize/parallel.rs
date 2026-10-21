//! Whole-document tokenisation: [`LineSource`], [`LineBytes`], [`TokenizedLines`] and [`tokenize_all`], with
//! parallel speculative chunks (§2.10.6).

use std::borrow::Cow;

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::error::InvariantViolation;
use rayon::prelude::*;

use super::{LineState, Token, Tokenizer};
use crate::compile::CompiledGrammar;

/// Lines per chunk (§2.10.6; also the cancellation check interval, §5.3).
const CHUNK_LINES: u32 = 8192;

/// Chunks per rayon task: each task creates one tokenizer and one token buffer, so this bounds the per-task
/// allocations to a fraction of a chunk's (the "≤ 4 allocations per chunk" gate, §6.4) while leaving enough
/// tasks to balance the load.
const MIN_CHUNKS_PER_TASK: usize = 4;

/// Random access to the lines of a document (content without EOL).
pub trait LineSource: Sync {
    /// Number of lines.
    fn line_count(&self) -> u32;
    /// Line `idx`.
    fn line(&self, idx: u32) -> Cow<'_, str>;
}

impl LineSource for [&str] {
    fn line_count(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
    fn line(&self, idx: u32) -> Cow<'_, str> {
        Cow::Borrowed(self.get(idx as usize).copied().unwrap_or(""))
    }
}

impl LineSource for [String] {
    fn line_count(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
    fn line(&self, idx: u32) -> Cow<'_, str> {
        Cow::Borrowed(self.get(idx as usize).map_or("", String::as_str))
    }
}

impl LineSource for Vec<&str> {
    fn line_count(&self) -> u32 {
        self.as_slice().line_count()
    }
    fn line(&self, idx: u32) -> Cow<'_, str> {
        self.as_slice().line(idx)
    }
}

impl LineSource for Vec<String> {
    fn line_count(&self) -> u32 {
        self.as_slice().line_count()
    }
    fn line(&self, idx: u32) -> Cow<'_, str> {
        self.as_slice().line(idx)
    }
}

/// The output bytes of one chunk: one arena plus one end offset per line, and the patch arena of the
/// speculative fix-up (lines `0..patched_prefix` live there).
#[derive(Debug, Clone, Default)]
struct ChunkBytes {
    bytes: Vec<u8>,
    ends: Vec<usize>,
    patched_prefix: u32,
    patch: Vec<u8>,
    patch_ends: Vec<usize>,
}

fn slice<'b>(bytes: &'b [u8], ends: &[usize], i: usize) -> &'b [u8] {
    let start = if i == 0 {
        0
    } else {
        ends.get(i - 1).copied().unwrap_or(0)
    };
    let end = ends.get(i).copied().unwrap_or(start);
    bytes.get(start..end).unwrap_or(&[])
}

impl ChunkBytes {
    fn get(&self, i: usize) -> &[u8] {
        if i < self.patched_prefix as usize {
            slice(&self.patch, &self.patch_ends, i)
        } else {
            slice(&self.bytes, &self.ends, i)
        }
    }
}

/// Per-line output bytes of `tokenize_all`, stored in one byte arena per chunk plus one end offset per line
/// (no per-line heap allocation; the 1M-line bench asserts the allocation count, §6.2.4). Lines re-tokenised
/// by the speculative fix-up (§2.10.6) live in a per-chunk patch arena.
///
/// Equality compares the logical content (the bytes of every line), not the arena layout, so a parallel run
/// with patched chunks equals the sequential run (INV-TOK-5).
#[derive(Debug, Clone, Default)]
pub struct LineBytes {
    chunks: Vec<ChunkBytes>,
    chunk_lines: u32,
    len: u32,
}

impl LineBytes {
    /// Number of lines.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// True when there are no lines.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes written for `line` (O(1)). Panics never: out-of-range lines give `&[]`.
    pub fn get(&self, line: u32) -> &[u8] {
        if line >= self.len || self.chunk_lines == 0 {
            return &[];
        }
        let chunk = (line / self.chunk_lines) as usize;
        let i = (line % self.chunk_lines) as usize;
        self.chunks.get(chunk).map_or(&[], |c| c.get(i))
    }

    /// All outputs in line order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> + '_ {
        (0..self.len).map(|line| self.get(line))
    }

    /// Number of lines whose output lives in a patch arena (lines re-tokenised by the speculative fix-up).
    pub fn patched_lines(&self) -> u32 {
        self.chunks.iter().map(|c| c.patched_prefix).sum()
    }
}

impl PartialEq for LineBytes {
    fn eq(&self, other: &LineBytes) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl Eq for LineBytes {}

/// Output of `tokenize_all`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenizedLines {
    /// The callback's output bytes per line.
    pub bytes: LineBytes,
    /// Tokenizer state at the start of each line (random-access re-tokenisation for views); empty when
    /// `!grammar.has_multiline()` (every line then starts in `Normal`).
    pub start_states: Vec<LineState>,
    /// Total regex runtime-limit hits.
    pub regex_errors: u64,
}

impl TokenizedLines {
    /// Checks the shape of the result (T4): one start state per line, or none at all (grammars without
    /// multi-line items), and the first line starts in `Normal`.
    pub fn check_invariants(&self) -> Result<(), InvariantViolation> {
        if !self.start_states.is_empty() && self.start_states.len() != self.bytes.len() as usize {
            return Err(InvariantViolation {
                id: "INV-TOK-4",
                detail: format!(
                    "{} start states for {} lines",
                    self.start_states.len(),
                    self.bytes.len()
                ),
            });
        }
        if self
            .start_states
            .first()
            .is_some_and(|s| *s != LineState::Normal)
        {
            return Err(InvariantViolation {
                id: "INV-TOK-4",
                detail: "the first line does not start in Normal".into(),
            });
        }
        Ok(())
    }
}

/// Tokenizes every line and calls `f(line_no, text, tokens, out)`, where `out` is the arena of the line's
/// chunk: `f` appends the line's output bytes (hikasa-text appends the line key, §2.13.2) and must not modify
/// earlier bytes. With a pool, uses speculative chunking (§2.10.6). The result is identical to the sequential
/// run (INV-TOK-5, T2).
///
/// Cancellation is checked once per chunk of 8192 lines, and inside each line every 64 KiB of scanned text
/// and before every regex engine call, so that one multi-megabyte line cannot delay it (§5.3, G4).
///
/// Progress: there is no progress parameter (§1.4.4); hikasa-text reports the `tokenizing` phase from its
/// callback, which receives every line number (§5.4).
pub fn tokenize_all<F>(
    grammar: &CompiledGrammar,
    lines: &dyn LineSource,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
    f: F,
) -> Result<TokenizedLines, Cancelled>
where
    F: Fn(u32, &str, &[Token], &mut Vec<u8>) + Sync,
{
    tokenize_all_with_chunk(grammar, lines, pool, cancel, CHUNK_LINES, f)
}

/// [`tokenize_all`] with an explicit chunk size (tests force tiny chunks to exercise the speculative fix-up,
/// P-TOK-5). `chunk_lines = 0` is treated as 1.
#[doc(hidden)]
pub fn tokenize_all_with_chunk<F>(
    grammar: &CompiledGrammar,
    lines: &dyn LineSource,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
    chunk_lines: u32,
    f: F,
) -> Result<TokenizedLines, Cancelled>
where
    F: Fn(u32, &str, &[Token], &mut Vec<u8>) + Sync,
{
    let n = lines.line_count();
    let c = chunk_lines.max(1);
    let n_chunks = n.div_ceil(c) as usize;
    let multiline = grammar.has_multiline();
    let mut start_states = if multiline {
        vec![LineState::Normal; n as usize]
    } else {
        Vec::new()
    };
    let job = Job {
        grammar,
        lines,
        chunk_lines: c,
        n,
        f: &f,
    };
    let mut outs: Vec<ChunkOut> = Vec::with_capacity(n_chunks);
    match pool {
        None => {
            let mut tok = Tokenizer::new(grammar);
            let mut tokens = Vec::with_capacity(64);
            let mut state = LineState::Normal;
            let mut state_chunks = start_states.chunks_mut(c as usize);
            for k in 0..n_chunks {
                cancel.check()?;
                let out =
                    job.run_chunk(&mut tok, &mut tokens, k, state, state_chunks.next(), cancel)?;
                state = out.end_state;
                outs.push(out);
            }
        }
        Some(pool) => {
            let init = || (Tokenizer::new(grammar), Vec::with_capacity(64));
            let results: Vec<Result<ChunkOut, Cancelled>> = pool.install(|| {
                if multiline {
                    start_states
                        .par_chunks_mut(c as usize)
                        .enumerate()
                        .with_min_len(MIN_CHUNKS_PER_TASK)
                        .map_init(init, |(tok, tokens), (k, states)| {
                            cancel.check()?;
                            job.run_chunk(tok, tokens, k, LineState::Normal, Some(states), cancel)
                        })
                        .collect()
                } else {
                    (0..n_chunks)
                        .into_par_iter()
                        .with_min_len(MIN_CHUNKS_PER_TASK)
                        .map_init(init, |(tok, tokens), k| {
                            cancel.check()?;
                            job.run_chunk(tok, tokens, k, LineState::Normal, None, cancel)
                        })
                        .collect()
                }
            });
            for r in results {
                outs.push(r?);
            }
            if multiline {
                job.fix_up(&mut outs, &mut start_states, cancel)?;
            }
        }
    }
    let regex_errors = outs.iter().map(|o| o.errors_total).sum();
    let result = TokenizedLines {
        bytes: LineBytes {
            chunks: outs.into_iter().map(|o| o.bytes).collect(),
            chunk_lines: c,
            len: n,
        },
        start_states,
        regex_errors,
    };
    debug_assert_eq!(result.check_invariants(), Ok(()));
    Ok(result)
}

/// One chunk's phase-1 result.
#[derive(Debug)]
struct ChunkOut {
    bytes: ChunkBytes,
    end_state: LineState,
    /// `(line in chunk, count)` for lines with regex runtime errors.
    errors: Vec<(u32, u32)>,
    errors_total: u64,
}

/// The shared, read-only inputs of one `tokenize_all` run.
struct Job<'a, F> {
    grammar: &'a CompiledGrammar,
    lines: &'a dyn LineSource,
    chunk_lines: u32,
    n: u32,
    f: &'a F,
}

impl<F> Job<'_, F>
where
    F: Fn(u32, &str, &[Token], &mut Vec<u8>) + Sync,
{
    /// Lines `[first, first + count)` of chunk `k`.
    fn range(&self, k: usize) -> (u32, usize) {
        let first = (k as u64 * u64::from(self.chunk_lines)).min(u64::from(self.n)) as u32;
        let count = self.chunk_lines.min(self.n - first) as usize;
        (first, count)
    }

    fn run_chunk(
        &self,
        tok: &mut Tokenizer<'_>,
        tokens: &mut Vec<Token>,
        k: usize,
        start: LineState,
        mut states: Option<&mut [LineState]>,
        cancel: &CancelToken,
    ) -> Result<ChunkOut, Cancelled> {
        let (first, count) = self.range(k);
        // Output size is unknown up front: start with 128 B per line for the first 64 lines, then reserve the
        // rest of the chunk from the average of the lines since the previous checkpoint (lines 64, 128, 256,
        // …, 4096) whenever that projection exceeds the capacity, with 1/8 slack. A chunk that starts with
        // blank lines is re-estimated as soon as its real lines show up, instead of growing its arena by
        // doubling (≤ 4 allocations per chunk, §6.4).
        let mut bytes = Vec::with_capacity(count.min(64) * 128);
        let mut ends = Vec::with_capacity(count);
        let mut errors = Vec::new();
        let mut errors_total = 0u64;
        let mut state = start;
        let mut checkpoint = 64usize;
        let (mut last_line, mut last_len) = (0usize, 0usize);
        for i in 0..count {
            let line_no = first + i as u32;
            let text = self.lines.line(line_no);
            if let Some(slot) = states.as_deref_mut().and_then(|s| s.get_mut(i)) {
                *slot = state;
            }
            let outcome = tok.tokenize_line_cancellable(&text, line_no, state, tokens, cancel)?;
            if outcome.regex_errors > 0 {
                errors.push((i as u32, outcome.regex_errors));
                errors_total += u64::from(outcome.regex_errors);
            }
            (self.f)(line_no, &text, tokens, &mut bytes);
            ends.push(bytes.len());
            state = outcome.end_state;
            if i + 1 == checkpoint && count > checkpoint {
                let recent = (bytes.len() - last_len) / (checkpoint - last_line) + 1;
                let need = bytes.len() + recent * (count - checkpoint);
                if need > bytes.capacity() {
                    let want = bytes.len() + recent * (count - checkpoint) * 9 / 8 + 256;
                    bytes.reserve_exact(want - bytes.len());
                }
                last_line = checkpoint;
                last_len = bytes.len();
                checkpoint = if checkpoint < 4096 {
                    checkpoint * 2
                } else {
                    usize::MAX
                };
            }
        }
        Ok(ChunkOut {
            bytes: ChunkBytes {
                bytes,
                ends,
                ..ChunkBytes::default()
            },
            end_state: state,
            errors,
            errors_total,
        })
    }

    /// Phase 2 of §2.10.6: re-tokenise each chunk's prefix from the true start state until a line's recomputed
    /// start state equals its speculative one.
    fn fix_up(
        &self,
        outs: &mut [ChunkOut],
        start_states: &mut [LineState],
        cancel: &CancelToken,
    ) -> Result<(), Cancelled> {
        let mut tok = Tokenizer::new(self.grammar);
        let mut tokens = Vec::with_capacity(64);
        let mut prev_end = LineState::Normal;
        for (k, out) in outs.iter_mut().enumerate() {
            if k > 0 && prev_end != LineState::Normal {
                cancel.check()?;
                let (first, count) = self.range(k);
                let base = first as usize;
                let mut state = prev_end;
                let mut patch = Vec::new();
                let mut patch_ends = Vec::new();
                let mut recomputed_errors = 0u64;
                let mut i = 0usize;
                while i < count {
                    let Some(slot) = start_states.get_mut(base + i) else {
                        break;
                    };
                    if *slot == state {
                        break;
                    }
                    *slot = state;
                    let line_no = first + i as u32;
                    let text = self.lines.line(line_no);
                    let outcome =
                        tok.tokenize_line_cancellable(&text, line_no, state, &mut tokens, cancel)?;
                    recomputed_errors += u64::from(outcome.regex_errors);
                    (self.f)(line_no, &text, &tokens, &mut patch);
                    patch_ends.push(patch.len());
                    state = outcome.end_state;
                    i += 1;
                }
                let removed: u64 = out
                    .errors
                    .iter()
                    .filter(|(line, _)| (*line as usize) < i)
                    .map(|(_, n)| u64::from(*n))
                    .sum();
                out.errors_total = out.errors_total - removed + recomputed_errors;
                out.bytes.patched_prefix = i as u32;
                out.bytes.patch = patch;
                out.bytes.patch_ends = patch_ends;
                if i == count {
                    out.end_state = state;
                }
            }
            prev_end = out.end_state;
        }
        Ok(())
    }
}
