//! Find of text or hex bytes (HX-005; D-334): [`HexPattern`], [`parse_hex_bytes`], [`HexFinder`].
//!
//! A pattern compiles to one matcher per byte (exact, ASCII letter in either case, `??` wildcard). The search
//! runs over the chunk visitor's chunks: each chunk reports the hits that lie inside it (candidates from the
//! longest exact run through `memchr::memmem`, or from the first case-folded letter through `memchr2`, then a
//! verify), and the `pattern_len − 1` bytes at each end; a hit across a chunk edge is found once in the seam.
//! Hits come in offset order and may overlap (`aa` in `aaa` gives 0 and 1). Replace needs editing (HX-006, P1).

use std::ops::{ControlFlow, Range};

use hikasa_core::{Ctl, Phase};
use hikasa_vfs::{StreamOptions, visit_chunks};
use serde::{Deserialize, Serialize};

use crate::align::{PhaseSink, windows};
use crate::document::HexDocument;
use crate::error::HexError;
use crate::vocab::{TextEncoding, VocabError};

/// The longest pattern in bytes (4,096).
pub const MAX_PATTERN: usize = 4096;

/// What to find.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexPattern {
    /// Text in an encoding; without `match_case`, ASCII letters match in either case.
    Text {
        /// The text.
        text: String,
        /// Its encoding (default UTF-8).
        encoding: TextEncoding,
        /// Match the case of ASCII letters (default false).
        match_case: bool,
    },
    /// Bytes; `None` is a `??` wildcard.
    Bytes {
        /// The bytes.
        bytes: Vec<Option<u8>>,
    },
}

/// Search direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Towards the end.
    Forward,
    /// Towards the start.
    Backward,
}

/// Parses hex bytes: `"DE AD ?? ef"`, `"dead??ef"`, `"0xDE,0xAD"` (separators: spaces, commas, `-`, `:`; `0x`
/// prefixes; `??` is a wildcard byte).
///
/// # Errors
///
/// [`VocabError`] for an odd number of digits, a non-hex character, or nothing.
pub fn parse_hex_bytes(text: &str) -> Result<Vec<Option<u8>>, VocabError> {
    let expected = "hex bytes such as \"DE AD ?? EF\"";
    let err = || VocabError::new("hex bytes", text, expected);
    let mut out = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, ',' | '-' | ':')) {
        let lower = token.to_ascii_lowercase();
        let digits = lower.strip_prefix("0x").unwrap_or(&lower);
        let chars: Vec<char> = digits.chars().collect();
        if !chars.len().is_multiple_of(2) {
            return Err(err());
        }
        for pair in chars.chunks(2) {
            match pair {
                ['?', '?'] => out.push(None),
                [h, l] => {
                    let (Some(h), Some(l)) = (h.to_digit(16), l.to_digit(16)) else {
                        return Err(err());
                    };
                    out.push(Some((h * 16 + l) as u8));
                }
                _ => return Err(err()),
            }
        }
    }
    if out.is_empty() {
        return Err(err());
    }
    Ok(out)
}

/// One byte of a compiled pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Matcher {
    Exact(u8),
    /// An ASCII letter: either case.
    Fold(u8, u8),
    Any,
}

impl Matcher {
    fn matches(self, b: u8) -> bool {
        match self {
            Matcher::Exact(x) => b == x,
            Matcher::Fold(x, y) => b == x || b == y,
            Matcher::Any => true,
        }
    }
}

/// How candidates are found.
#[derive(Debug, Clone)]
enum Anchor {
    /// The longest exact run at this pattern offset.
    Literal { at: usize, bytes: Vec<u8> },
    /// A case-folded letter at this offset.
    Fold { at: usize, a: u8, b: u8 },
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct HexFinder {
    matchers: Vec<Matcher>,
    anchor: Anchor,
}

/// What a range search wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    First,
    Last,
    All(u64),
}

/// One chunk's hits and edges.
#[derive(Debug, Default)]
struct ChunkHits {
    hits: Vec<u64>,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl HexFinder {
    /// Compiles a pattern.
    ///
    /// # Errors
    ///
    /// `invalid_input` ([`VocabError`]) for an empty pattern, one over 4,096 bytes, an all-wildcard pattern, or
    /// text the encoding cannot write (Latin-1 above U+00FF).
    pub fn new(pattern: &HexPattern) -> Result<HexFinder, VocabError> {
        let matchers: Vec<Matcher> = match pattern {
            HexPattern::Bytes { bytes } => bytes
                .iter()
                .map(|b| b.map_or(Matcher::Any, Matcher::Exact))
                .collect(),
            HexPattern::Text {
                text,
                encoding,
                match_case,
            } => {
                let bytes = encode(text, *encoding)?;
                let letter_byte = |i: usize| match encoding {
                    // In UTF-16 only the low byte of an ASCII code unit is a letter.
                    TextEncoding::Utf16Le => i.is_multiple_of(2) && bytes.get(i + 1) == Some(&0),
                    TextEncoding::Utf16Be => i % 2 == 1 && bytes.get(i - 1) == Some(&0),
                    TextEncoding::Utf8 | TextEncoding::Latin1 => true,
                };
                bytes
                    .iter()
                    .enumerate()
                    .map(|(i, &b)| {
                        if !match_case && b.is_ascii_alphabetic() && letter_byte(i) {
                            Matcher::Fold(b.to_ascii_lowercase(), b.to_ascii_uppercase())
                        } else {
                            Matcher::Exact(b)
                        }
                    })
                    .collect()
            }
        };
        let refuse = |what: &str| VocabError::new("pattern", &describe(pattern), what);
        if matchers.is_empty() {
            return Err(refuse("a non-empty pattern"));
        }
        if matchers.len() > MAX_PATTERN {
            return Err(refuse("at most 4096 bytes"));
        }
        let anchor = anchor(&matchers).ok_or_else(|| refuse("at least one byte that is not ??"))?;
        Ok(HexFinder { matchers, anchor })
    }

    /// Pattern length in bytes.
    pub fn len(&self) -> usize {
        self.matchers.len()
    }

    /// Never true (an empty pattern is refused).
    pub fn is_empty(&self) -> bool {
        self.matchers.is_empty()
    }

    /// True when the pattern matches at `hay[0..]`.
    fn verify(&self, hay: &[u8]) -> bool {
        hay.len() >= self.matchers.len()
            && self.matchers.iter().zip(hay).all(|(m, &b)| m.matches(b))
    }

    /// Hits starting in `hay` that lie wholly inside it (offsets `base + i`), as `want` asks.
    fn find_in(&self, hay: &[u8], base: u64, want: Want, out: &mut Vec<u64>) {
        let plen = self.matchers.len();
        if hay.len() < plen {
            return;
        }
        let last_start = hay.len() - plen;
        let push = |start: usize, out: &mut Vec<u64>| -> bool {
            if want == Want::Last {
                out.clear();
            }
            out.push(base + start as u64);
            match want {
                Want::First => false,
                Want::Last => true,
                Want::All(limit) => (out.len() as u64) <= limit,
            }
        };
        match &self.anchor {
            Anchor::Literal { at, bytes } => {
                let finder = memchr::memmem::Finder::new(bytes);
                let mut i = *at;
                while let Some(p) = finder.find(&hay[i.min(hay.len())..]) {
                    let pos = i + p;
                    let start = pos - at;
                    if start > last_start {
                        break;
                    }
                    if self.verify(&hay[start..]) && !push(start, out) {
                        return;
                    }
                    i = pos + 1;
                }
            }
            Anchor::Fold { at, a, b } => {
                let mut i = *at;
                while i < hay.len() {
                    let Some(p) = memchr::memchr2(*a, *b, &hay[i..]) else {
                        break;
                    };
                    let pos = i + p;
                    let start = pos - at;
                    if start > last_start {
                        break;
                    }
                    if self.verify(&hay[start..]) && !push(start, out) {
                        return;
                    }
                    i = pos + 1;
                }
            }
        }
    }

    /// Searches `range` of `doc`; the hits wholly inside it, as `want` asks; also whether `All`'s limit was
    /// passed.
    fn search(
        &self,
        doc: &HexDocument,
        range: Range<u64>,
        want: Want,
        stream: &StreamOptions,
        ctl: &Ctl<'_>,
    ) -> Result<Vec<u64>, HexError> {
        let edge = self.matchers.len() - 1;
        let f = |offset: u64, bytes: &[u8]| {
            let mut hits = Vec::new();
            self.find_in(bytes, offset, want, &mut hits);
            let found = !hits.is_empty();
            let value = ChunkHits {
                hits,
                head: bytes[..edge.min(bytes.len())].to_vec(),
                tail: bytes[bytes.len() - edge.min(bytes.len())..].to_vec(),
            };
            if want == Want::First && found {
                ControlFlow::Break(value)
            } else {
                ControlFlow::Continue(value)
            }
        };
        let sink = PhaseSink::new(ctl, Phase::Searching, range.end - range.start);
        let inner = Ctl {
            cancel: ctl.cancel,
            progress: &sink,
        };
        let mut all = Vec::new();
        for window in windows(range.end - range.start, stream.effective_threads()) {
            let window = range.start + window.start..range.start + window.end;
            sink.base(window.start - range.start);
            let visit = visit_chunks(doc.source(), window.clone(), stream, &inner, &f)
                .map_err(|e| HexError::from_vfs(doc.display(), e))?;
            let mut previous: Option<(u64, Vec<u8>)> = None;
            let mut chunk_start = window.start;
            for chunk in visit.values {
                // The seam before this chunk: hits that start in the previous chunk's tail.
                if let Some((tail_start, tail)) = previous.take() {
                    let mut seam = tail;
                    seam.extend_from_slice(&chunk.head);
                    self.find_in(&seam, tail_start, Want::All(u64::MAX), &mut all);
                }
                all.extend(chunk.hits);
                let chunk_len = hikasa_vfs::VISIT_CHUNK.min(window.end - chunk_start);
                let chunk_end = chunk_start + chunk_len;
                previous = Some((chunk_end - chunk.tail.len() as u64, chunk.tail));
                chunk_start = chunk_end;
                if want == Want::First && !all.is_empty() {
                    break;
                }
            }
            // The seam into the next window.
            if let Some((tail_start, tail)) = previous
                && window.end < range.end
                && !(want == Want::First && !all.is_empty())
            {
                let mut seam = tail;
                let mut head = vec![0u8; edge.min((range.end - window.end) as usize)];
                let n = doc.read(window.end, &mut head)?;
                seam.extend_from_slice(&head[..n]);
                self.find_in(&seam, tail_start, Want::All(u64::MAX), &mut all);
            }
            all.sort_unstable();
            all.dedup();
            match want {
                Want::First if !all.is_empty() => {
                    all.truncate(1);
                    return Ok(all);
                }
                Want::All(limit) if all.len() as u64 > limit => {
                    all.truncate(limit as usize + 1);
                    return Ok(all);
                }
                _ => {}
            }
        }
        if want == Want::Last
            && let Some(&last) = all.last()
        {
            return Ok(vec![last]);
        }
        Ok(all)
    }

    /// The next hit from `from` in `dir`: forward, the first hit starting at or after `from`; backward, the last
    /// hit starting before `from`. With `wrap`, the search continues from the other end. Returns the hit's byte
    /// range.
    ///
    /// # Errors
    ///
    /// A read error, [`HexError::Truncated`], cancellation.
    pub fn next(
        &self,
        doc: &HexDocument,
        from: u64,
        dir: Direction,
        wrap: bool,
        stream: &StreamOptions,
        ctl: &Ctl<'_>,
    ) -> Result<Option<Range<u64>>, HexError> {
        let len = doc.len();
        let plen = self.matchers.len() as u64;
        let from = from.min(len);
        let hit = |at: u64| at..at + plen;
        match dir {
            Direction::Forward => {
                if let Some(&at) = self
                    .search(doc, from..len, Want::First, stream, ctl)?
                    .first()
                {
                    return Ok(Some(hit(at)));
                }
                if wrap {
                    let end = from.saturating_add(plen - 1).min(len);
                    let found = self.search(doc, 0..end, Want::First, stream, ctl)?;
                    return Ok(found.first().map(|&at| hit(at)));
                }
                Ok(None)
            }
            Direction::Backward => {
                let end = from.saturating_add(plen - 1).min(len);
                if let Some(&at) = self
                    .search(doc, 0..end, Want::Last, stream, ctl)?
                    .iter()
                    .rev()
                    .find(|&&at| at < from)
                {
                    return Ok(Some(hit(at)));
                }
                if wrap {
                    let found = self.search(doc, from..len, Want::Last, stream, ctl)?;
                    return Ok(found.last().map(|&at| hit(at)));
                }
                Ok(None)
            }
        }
    }

    /// Every hit's offset in order, at most `limit`; `true` when the limit was reached (more hits exist).
    ///
    /// # Errors
    ///
    /// As [`next`](Self::next).
    pub fn all(
        &self,
        doc: &HexDocument,
        limit: u64,
        stream: &StreamOptions,
        ctl: &Ctl<'_>,
    ) -> Result<(Vec<u64>, bool), HexError> {
        let mut hits = self.search(doc, 0..doc.len(), Want::All(limit), stream, ctl)?;
        let reached = hits.len() as u64 > limit;
        hits.truncate(limit as usize);
        Ok((hits, reached))
    }
}

/// A short description of a pattern for messages.
fn describe(pattern: &HexPattern) -> String {
    match pattern {
        HexPattern::Text { text, .. } => text.clone(),
        HexPattern::Bytes { bytes } => bytes
            .iter()
            .map(|b| b.map_or_else(|| "??".to_owned(), |b| format!("{b:02X}")))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// The text's bytes in `encoding`.
fn encode(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, VocabError> {
    Ok(match encoding {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Latin1 => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).ok())
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| {
                VocabError::new("pattern", text, "text Latin-1 can write (up to U+00FF)")
            })?,
        TextEncoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        TextEncoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    })
}

/// The longest exact run, else the first case-folded letter; `None` for an all-wildcard pattern.
fn anchor(matchers: &[Matcher]) -> Option<Anchor> {
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < matchers.len() {
        if let Matcher::Exact(_) = matchers[i] {
            let start = i;
            while i < matchers.len() && matches!(matchers[i], Matcher::Exact(_)) {
                i += 1;
            }
            if best.is_none_or(|(_, len)| i - start > len) {
                best = Some((start, i - start));
            }
        } else {
            i += 1;
        }
    }
    if let Some((at, len)) = best {
        let bytes = matchers[at..at + len]
            .iter()
            .map(|m| match m {
                Matcher::Exact(b) => *b,
                _ => 0,
            })
            .collect();
        return Some(Anchor::Literal { at, bytes });
    }
    matchers.iter().enumerate().find_map(|(at, m)| match m {
        Matcher::Fold(a, b) => Some(Anchor::Fold { at, a: *a, b: *b }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_spellings() {
        let dead = vec![Some(0xDE), Some(0xAD), None, Some(0xEF)];
        assert_eq!(parse_hex_bytes("DE AD ?? ef"), Ok(dead.clone()));
        assert_eq!(parse_hex_bytes("dead??ef"), Ok(dead));
        assert_eq!(
            parse_hex_bytes("0xDE,0xAD"),
            Ok(vec![Some(0xDE), Some(0xAD)])
        );
        for bad in ["", "  ", "d", "dex", "g0", "?a"] {
            assert!(parse_hex_bytes(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn refused_patterns() {
        let bytes = |b: Vec<Option<u8>>| HexPattern::Bytes { bytes: b };
        assert!(HexFinder::new(&bytes(vec![])).is_err());
        assert!(HexFinder::new(&bytes(vec![None, None])).is_err());
        assert!(HexFinder::new(&bytes(vec![Some(1); 4097])).is_err());
        assert!(HexFinder::new(&bytes(vec![Some(1); 4096])).is_ok());
        let text = HexPattern::Text {
            text: "ā".into(),
            encoding: TextEncoding::Latin1,
            match_case: false,
        };
        assert!(HexFinder::new(&text).is_err());
    }
}
