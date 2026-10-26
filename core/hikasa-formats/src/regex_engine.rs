//! User patterns: [`Pattern`], [`EngineKind`] and [`RegexLimits`] (§2.11).
//!
//! The linear-time `regex` crate is always tried first; `fancy-regex` is reached only for patterns that
//! `regex` rejects (backreferences, look-around). Both engines see the whole line as the haystack, so
//! `find_at(line, p)` evaluates `^`, `$`, `\b` and look-behind against the real line context; `^` means line
//! start.
//!
//! The module also holds the conservative first-character analysis used by the tokenizer's prefilter
//! (§2.10.5): it may claim that a pattern can start with more characters than it really can, never fewer.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::error::PatternError;

mod simple;

/// Regex resource limits (user patterns are untrusted input; T9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegexLimits {
    /// `regex` compiled size limit in bytes. Default 8 MiB.
    pub size_limit: usize,
    /// `fancy-regex` backtracking step limit. Default 1_000_000 (exceeding it = no match + warning).
    pub backtrack_limit: usize,
}

impl Default for RegexLimits {
    /// 8 MiB compiled size, 1,000,000 backtracking steps.
    fn default() -> RegexLimits {
        RegexLimits {
            size_limit: 8 << 20,
            backtrack_limit: 1_000_000,
        }
    }
}

/// Which engine compiled a pattern (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// The linear-time `regex` crate.
    Standard,
    /// `fancy-regex` (backtracking; backreferences and look-around).
    Fancy,
}

#[derive(Debug, Clone)]
enum Engine {
    Standard(regex::Regex),
    Fancy(fancy_regex::Regex),
}

/// A compiled user pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    source: String,
    case_insensitive: bool,
    engine: Engine,
    /// `^(?s:.)(?:source)` for anchored matches of a `regex` pattern at `p > 0`: it runs on the haystack from the
    /// char before `p`, so `\b`, `^` and `(?m)^` still see their real one-char left context (the `regex` crate
    /// has no anchored search at an offset). `None` when the wrapped form does not compile (for example an
    /// `(?x)` comment that would swallow the closing parenthesis); [`Pattern::match_anchored`] then falls back to
    /// an unanchored search.
    anchored: Option<regex::Regex>,
    /// Exact ASCII accelerator for anchored matches of simple `regex` patterns (§2.10.5).
    simple: Option<simple::Simple>,
}

/// The answer of the simple-pattern accelerator at one position (§2.10.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accel {
    /// Decided: the leftmost-first match starting exactly at `p` ends at `end` (`None`: no match starts at
    /// `p`; the end may equal `p` for an empty match). `examined` = bytes looked at from `p` on.
    Decided {
        /// End of the match at `p`.
        end: Option<usize>,
        /// Bytes examined from `p` on (the cost of the decision).
        examined: usize,
    },
    /// Not decided (non-ASCII input, or a step or scan budget ran out): ask the regex engine.
    Undecided,
}

impl Pattern {
    /// Compiles: `regex` first; when `regex` rejects the pattern, `fancy-regex` is tried (§2.11).
    pub fn compile(
        source: &str,
        case_insensitive: bool,
        limits: &RegexLimits,
    ) -> Result<Pattern, PatternError> {
        let standard = regex::RegexBuilder::new(source)
            .case_insensitive(case_insensitive)
            .size_limit(limits.size_limit)
            .build();
        let e1 = match standard {
            Ok(re) => {
                let anchored = regex::RegexBuilder::new(&format!("^(?s:.)(?:{source})"))
                    .case_insensitive(case_insensitive)
                    .size_limit(limits.size_limit)
                    .build()
                    .ok();
                return Ok(Pattern {
                    source: source.to_owned(),
                    case_insensitive,
                    engine: Engine::Standard(re),
                    anchored,
                    simple: simple::Simple::compile(source, case_insensitive),
                });
            }
            Err(e) => one_line(&e.to_string()),
        };
        let fancy_source = if case_insensitive {
            format!("(?i){source}")
        } else {
            source.to_owned()
        };
        let fancy = fancy_regex::RegexBuilder::new(&fancy_source)
            .backtrack_limit(limits.backtrack_limit)
            .delegate_size_limit(limits.size_limit)
            .build();
        match fancy {
            Ok(re) => Ok(Pattern {
                source: source.to_owned(),
                case_insensitive,
                engine: Engine::Fancy(re),
                anchored: None,
                simple: None,
            }),
            Err(e2) => Err(PatternError {
                pattern: source.to_owned(),
                message: format!("{e1}; {}", one_line(&e2.to_string())),
                runtime: false,
            }),
        }
    }

    /// Engine used.
    pub fn engine(&self) -> EngineKind {
        match self.engine {
            Engine::Standard(_) => EngineKind::Standard,
            Engine::Fancy(_) => EngineKind::Fancy,
        }
    }

    /// The pattern as written.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when the pattern was compiled case-insensitively.
    pub fn case_insensitive(&self) -> bool {
        self.case_insensitive
    }

    /// The leftmost-first match that starts exactly at `p`, decided without the regex engine when the pattern
    /// is simple, the examined input is ASCII and the scan stays within its budget; `None` when the pattern has
    /// no accelerator.
    #[inline]
    pub(crate) fn accelerate(&self, haystack: &str, p: usize) -> Option<Accel> {
        let simple = self.simple.as_ref()?;
        Some(match simple.match_at(haystack, p) {
            Ok((end, examined)) => Accel::Decided { end, examined },
            Err(simple::Undecided) => Accel::Undecided,
        })
    }

    /// The end of the leftmost-first match that starts exactly at `p` (an anchored match; the end may equal
    /// `p` for an empty match), with the whole haystack as context: `^`, `\b` and look-behind see the text
    /// before `p`, `$` and look-ahead the text after the match. `Ok(None)` when no match starts at `p` (also
    /// for a `p` past the end or inside a char); `Err` only for fancy-regex runtime limits.
    ///
    /// This is exactly "the match found by `find_at(haystack, p)` starts at `p`" (§2.10.2), decided without
    /// scanning past the match: a leftmost search that starts at `p` returns a match at `p` whenever one
    /// exists, and with the same end.
    pub(crate) fn match_anchored(
        &self,
        haystack: &str,
        p: usize,
    ) -> Result<Option<usize>, PatternError> {
        if p > haystack.len() || !haystack.is_char_boundary(p) {
            return Ok(None);
        }
        match &self.engine {
            Engine::Standard(re) => {
                let before = haystack
                    .get(..p)
                    .and_then(|s| s.chars().next_back())
                    .map_or(0, char::len_utf8);
                match (&self.anchored, haystack.get(p - before..)) {
                    (Some(anchored), Some(shifted)) if before > 0 => {
                        Ok(anchored.find(shifted).map(|m| p - before + m.end()))
                    }
                    _ => Ok(re
                        .find_at(haystack, p)
                        .filter(|m| m.start() == p)
                        .map(|m| m.end())),
                }
            }
            Engine::Fancy(re) => {
                let input = fancy_regex::RegexInput::new(haystack)
                    .from_pos(p)
                    .anchored(true);
                match re.find_input(input) {
                    Ok(m) => Ok(m.filter(|m| m.start() == p).map(|m| m.end())),
                    Err(e) => Err(PatternError {
                        pattern: self.source.clone(),
                        message: one_line(&e.to_string()),
                        runtime: true,
                    }),
                }
            }
        }
    }

    /// True when anchored matches at a position are decided by the exact ASCII accelerator for simple patterns
    /// (§2.10.5) before the regex engine is asked.
    pub fn is_accelerated(&self) -> bool {
        self.simple.is_some()
    }

    /// Leftmost-first match starting at or after byte `start` (`^`, `\b` see the full haystack).
    /// `Err` only for fancy-regex runtime limits. A `start` past the end or inside a char gives `Ok(None)`.
    pub fn find_at(
        &self,
        haystack: &str,
        start: usize,
    ) -> Result<Option<Range<usize>>, PatternError> {
        if start > haystack.len() || !haystack.is_char_boundary(start) {
            return Ok(None);
        }
        match &self.engine {
            Engine::Standard(re) => Ok(re.find_at(haystack, start).map(|m| m.range())),
            Engine::Fancy(re) => match re.find_from_pos(haystack, start) {
                Ok(m) => Ok(m.map(|m| m.start()..m.end())),
                Err(e) => Err(PatternError {
                    pattern: self.source.clone(),
                    message: one_line(&e.to_string()),
                    runtime: true,
                }),
            },
        }
    }
}

/// Joins a multi-line engine message (with its caret diagram) into one line.
fn one_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------------------------------
// First-character analysis (prefilter, §2.10.5)
// ---------------------------------------------------------------------------------------------------------

/// Characters that can start a non-empty match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FirstChars {
    /// ASCII characters (bit `b` = byte `b`).
    pub ascii: u128,
    /// Some non-ASCII character may start a match.
    pub non_ascii: bool,
    /// Any character may start a match at line position 0 (a branch anchored with `^`).
    pub at_line_start: bool,
}

impl FirstChars {
    /// Every character, everywhere.
    pub(crate) const ANY: FirstChars = FirstChars {
        ascii: u128::MAX,
        non_ascii: true,
        at_line_start: true,
    };

    fn add(&mut self, c: char) {
        if c.is_ascii() {
            self.ascii |= 1u128 << (c as u32);
        } else {
            self.non_ascii = true;
        }
    }

    fn add_range(&mut self, lo: char, hi: char) {
        let (lo, hi) = (lo as u32, hi as u32);
        for b in lo..=hi.min(127) {
            self.ascii |= 1u128 << b;
        }
        if hi >= 128 {
            self.non_ascii = true;
        }
    }

    pub(crate) fn union(&mut self, other: FirstChars) {
        self.ascii |= other.ascii;
        self.non_ascii |= other.non_ascii;
        self.at_line_start |= other.at_line_start;
    }

    /// Adds the other ASCII case of every ASCII letter. Case folding crosses the ASCII boundary in both
    /// directions: an ASCII letter may match a non-ASCII char (so `non_ascii` is set), and two non-ASCII chars
    /// fold onto ASCII letters — KELVIN SIGN U+212A onto `k` (in `regex` and in the simple lower-case mapping of
    /// case-insensitive literals) and LONG S U+017F onto `s` (in `regex`). So a set that admits some non-ASCII
    /// char also admits `k`, `K`, `s` and `S` (§2.10.5: the prefilter must never reject a possible match).
    pub(crate) fn fold_case(&mut self) {
        if self.non_ascii {
            for c in *b"kKsS" {
                self.ascii |= 1u128 << c;
            }
        }
        let mut add = 0u128;
        for b in b'a'..=b'z' {
            let lower = 1u128 << b;
            let upper = 1u128 << (b - 32);
            if self.ascii & (lower | upper) != 0 {
                add |= lower | upper;
            }
        }
        self.ascii |= add;
        self.non_ascii = true;
    }

    /// The first char of a literal (case-insensitive literals also admit the other case).
    pub(crate) fn of_literal(text: &str, case_insensitive: bool) -> FirstChars {
        let mut out = FirstChars::default();
        match text.chars().next() {
            Some(c) => out.add(c),
            None => return FirstChars::ANY,
        }
        if case_insensitive {
            out.fold_case();
        }
        out
    }
}

const DIGITS: u128 = 0x03FF << b'0';
fn word_ascii() -> u128 {
    let mut set = DIGITS | (1u128 << b'_');
    for b in b'a'..=b'z' {
        set |= 1u128 << b;
        set |= 1u128 << (b - 32);
    }
    set
}
const SPACE_ASCII: u128 = (1u128 << b'\t')
    | (1u128 << b'\n')
    | (1u128 << 0x0B)
    | (1u128 << 0x0C)
    | (1u128 << b'\r')
    | (1u128 << b' ');

/// Conservative analysis of the characters that can start a non-empty match of `source`; `None` means "any
/// character" (the analysis gave up). Handles top-level alternation, a leading literal, escape, `^` or simple
/// class, and gives up on groups, inline flags, dots, negated or nested classes and optional first atoms.
pub(crate) fn first_chars(source: &str, case_insensitive: bool) -> Option<FirstChars> {
    // Inline flags (`(?i)`, `(?x)`, …) may change how every later atom matches, even across `|`.
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '(' if chars.peek() == Some(&'?') => {
                chars.next();
                if !matches!(chars.peek(), Some(':' | '=' | '!' | '<')) {
                    return None;
                }
            }
            _ => {}
        }
    }
    let mut out = FirstChars::default();
    for branch in split_alternatives(source)? {
        out.union(first_of_branch(branch)?);
    }
    if case_insensitive {
        out.fold_case();
    }
    Some(out)
}

/// Splits at top-level `|` (outside groups and classes). `None` for nested classes.
fn split_alternatives(source: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut chars = source.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => skip_class(&mut chars)?,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                parts.push(&source[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&source[start..]);
    Some(parts)
}

/// Skips a class body after its `[`. `None` for nested classes.
fn skip_class(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Option<()> {
    if chars.peek().is_some_and(|&(_, c)| c == '^') {
        chars.next();
    }
    if chars.peek().is_some_and(|&(_, c)| c == ']') {
        chars.next();
    }
    while let Some((_, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => return None,
            ']' => return Some(()),
            _ => {}
        }
    }
    None
}

/// The text of a group after its `(`, up to the matching `)` (consumed). `None` when unbalanced or when it
/// contains a nested class.
fn group_body(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    let mut body = String::new();
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                body.push(c);
                body.push(chars.next()?);
            }
            '[' => {
                body.push(c);
                let mut first = true;
                loop {
                    let k = chars.next()?;
                    body.push(k);
                    match k {
                        '\\' => body.push(chars.next()?),
                        '[' => return None,
                        ']' if !first => break,
                        _ => {}
                    }
                    first = k == '^' && body.ends_with("[^");
                }
            }
            '(' => {
                depth += 1;
                body.push(c);
            }
            ')' if depth == 0 => return Some(body),
            ')' => {
                depth -= 1;
                body.push(c);
            }
            c => body.push(c),
        }
    }
    None
}

/// True when a group (given by its body) can only match the empty string: a look-around, or a non-capturing
/// group whose alternatives are each `^` or such a group. `None` when the body cannot be analysed.
fn zero_width(body: &str) -> Option<bool> {
    let is_look_behind = (body.starts_with("?<=") || body.starts_with("?<!")) && body.len() > 3;
    if is_look_behind || body.starts_with("?=") || body.starts_with("?!") {
        return Some(true);
    }
    let Some(inner) = body.strip_prefix("?:") else {
        return Some(false);
    };
    for alt in split_alternatives(inner)? {
        if alt == "^" {
            continue;
        }
        let mut chars = alt.chars().peekable();
        if chars.next() != Some('(') {
            return Some(false);
        }
        let nested = group_body(&mut chars)?;
        if chars.next().is_some() || !zero_width(&nested)? {
            return Some(false);
        }
    }
    Some(true)
}

fn first_of_branch(branch: &str) -> Option<FirstChars> {
    let mut chars = branch.chars().peekable();
    let mut out = FirstChars::default();
    // Atoms are added until one that must match: an optional atom (`?`, `*`, `{0,…}`) lets the next atom start
    // the match too. A branch made only of optional atoms matches empty (no token) or starts with one of them.
    loop {
        let Some(first) = chars.next() else {
            return Some(out);
        };
        match first {
            '^' => {
                out.at_line_start = true;
                return Some(out);
            }
            // A zero-width group (look-around, or `(?:^|(?<=\s))`) consumes nothing: the next atom starts the
            // match.
            '(' => {
                let body = group_body(&mut chars)?;
                if !zero_width(&body)? || matches!(chars.peek(), Some('?' | '*' | '+' | '{')) {
                    return None;
                }
                continue;
            }
            '$' | '.' | ')' | '*' | '+' | '?' | '{' | '|' => return None,
            '\\' => {
                let e = chars.next()?;
                match e {
                    'd' => {
                        out.ascii |= DIGITS;
                        out.non_ascii = true;
                    }
                    'w' => {
                        out.ascii |= word_ascii();
                        out.non_ascii = true;
                    }
                    's' => {
                        out.ascii |= SPACE_ASCII;
                        out.non_ascii = true;
                    }
                    'x' => out.add(hex_escape(&mut chars)?),
                    // `\<` and `\>` are word-boundary assertions outside classes.
                    '<' | '>' => return None,
                    _ => out.add(simple_escape(e)?),
                }
            }
            '[' => class(&mut chars, &mut out)?,
            c => out.add(c),
        }
        let optional = match chars.peek() {
            Some('?' | '*') => {
                chars.next();
                true
            }
            Some('{') => {
                chars.next();
                let mut body = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    body.push(c);
                }
                let min = body.split(',').next().unwrap_or("").trim();
                min.is_empty() || min.parse::<u32>().ok()? == 0
            }
            _ => false,
        };
        if !optional {
            return Some(out);
        }
        // A lazy marker after the quantifier.
        if chars.peek() == Some(&'?') {
            chars.next();
        }
    }
}

/// `\xHH` or `\x{H…}` after the `x`.
fn hex_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<char> {
    let mut digits = String::new();
    if chars.peek() == Some(&'{') {
        chars.next();
        for c in chars.by_ref() {
            if c == '}' {
                break;
            }
            digits.push(c);
        }
    } else {
        for _ in 0..2 {
            digits.push(chars.next()?);
        }
    }
    char::from_u32(u32::from_str_radix(&digits, 16).ok()?)
}

/// The char of a one-char escape (`\t`, `\.`, `\$` …); `None` for classes, assertions and unknown escapes.
fn simple_escape(e: char) -> Option<char> {
    match e {
        't' => Some('\t'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        'f' => Some('\x0C'),
        'v' => Some('\x0B'),
        'a' => Some('\x07'),
        c if c.is_ascii_punctuation() => Some(c),
        _ => None,
    }
}

/// A class after its `[` (non-negated, not nested, no set operations).
fn class(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut FirstChars) -> Option<()> {
    if chars.peek() == Some(&'^') {
        return None;
    }
    let mut first = true;
    let mut prev: Option<char> = None;
    loop {
        let c = chars.next()?;
        let member = match c {
            ']' if !first => return Some(()),
            '[' => return None,
            '&' | '~' if chars.peek() == Some(&c) => return None,
            '-' if !first && chars.peek() == Some(&'-') => return None,
            '\\' => {
                let e = chars.next()?;
                match e {
                    'd' => {
                        out.ascii |= DIGITS;
                        out.non_ascii = true;
                        None
                    }
                    'w' => {
                        out.ascii |= word_ascii();
                        out.non_ascii = true;
                        None
                    }
                    's' => {
                        out.ascii |= SPACE_ASCII;
                        out.non_ascii = true;
                        None
                    }
                    'x' => Some(hex_escape(chars)?),
                    _ => Some(simple_escape(e)?),
                }
            }
            '-' if prev.is_some() && chars.peek().is_some_and(|&n| n != ']') => {
                let lo = prev.take()?;
                let hi = match chars.next()? {
                    '\\' => {
                        let e = chars.next()?;
                        match e {
                            'x' => hex_escape(chars)?,
                            _ => simple_escape(e)?,
                        }
                    }
                    '[' => return None,
                    hi => hi,
                };
                if hi < lo {
                    return None;
                }
                out.add_range(lo, hi);
                first = false;
                continue;
            }
            c => Some(c),
        };
        first = false;
        if let Some(m) = member {
            out.add(m);
        }
        prev = member;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(src: &str) -> Pattern {
        Pattern::compile(src, false, &RegexLimits::default()).unwrap()
    }

    #[test]
    fn engine_selection() {
        assert_eq!(compile("a+").engine(), EngineKind::Standard);
        assert_eq!(compile(r"(a)\1").engine(), EngineKind::Fancy);
        assert_eq!(compile(r"(?<=x)y").engine(), EngineKind::Fancy);
        let err = Pattern::compile("(", false, &RegexLimits::default()).unwrap_err();
        assert!(!err.runtime);
        let (e1, e2) = err.message.split_once("; ").unwrap();
        assert!(e1.contains("unclosed group"), "{e1}");
        assert!(!e2.is_empty(), "{e2}");
        assert!(!err.message.contains('\n'));
    }

    #[test]
    fn find_at_sees_the_whole_line() {
        let p = compile(r"\bfoo");
        assert_eq!(p.find_at("xfoo foo", 1).unwrap(), Some(5..8));
        let caret = compile("^a");
        assert_eq!(caret.find_at("aa", 1).unwrap(), None);
        assert_eq!(caret.find_at("aa", 0).unwrap(), Some(0..1));
        let behind = compile(r"(?<=x)y");
        assert_eq!(behind.find_at("xy", 1).unwrap(), Some(1..2));
        assert_eq!(p.find_at("foo", 9).unwrap(), None);
        assert_eq!(p.find_at("é", 1).unwrap(), None);
        let ci = Pattern::compile("ab", true, &RegexLimits::default()).unwrap();
        assert_eq!(ci.find_at("xAB", 0).unwrap(), Some(1..3));
        let ci_fancy = Pattern::compile(r"(a)\1", true, &RegexLimits::default()).unwrap();
        assert_eq!(ci_fancy.engine(), EngineKind::Fancy);
        assert_eq!(ci_fancy.find_at("aA", 0).unwrap(), Some(0..2));
    }

    #[test]
    fn backtrack_limit_is_a_runtime_error() {
        let limits = RegexLimits {
            size_limit: 8 << 20,
            backtrack_limit: 100,
        };
        let p = Pattern::compile(r"(a*)*\1b", false, &limits).unwrap();
        let err = p.find_at(&"a".repeat(40), 0).unwrap_err();
        assert!(err.runtime);
    }

    /// The anchored decision equals "the leftmost match found by `find_at(h, p)` starts at `p`" (same end) for
    /// every position, including the one-char left context of `\b`, `\B`, `^` and `(?m)^`, case folding across
    /// the ASCII boundary, a wrapped `(?x)` comment (no anchored variant: the fallback), and fancy patterns.
    #[test]
    fn anchored_matches_agree_with_find_at() {
        let patterns = [
            (r"\bfoo", false),
            (r"\Bo+", false),
            ("^a", false),
            ("(?m)^b", false),
            ("a$", false),
            (r"\w+", false),
            ("k+", true),
            ("\u{17F}t", true),
            ("x*", false),
            ("(?x) a b # trailing comment", false),
            (r"(?<=a)b+", false),
            (r"[a-z]+(?=\s*=)", false),
            (r"(a)\1", true),
        ];
        let hays = [
            "xfoo foo fo",
            "a\nb ab",
            "kK\u{212A} st\u{17F}t ſt",
            "abba ab= x =",
            "",
            "é\u{212A}a ab",
            "aAaa b",
        ];
        for (src, ci) in patterns {
            let p = Pattern::compile(src, ci, &RegexLimits::default()).unwrap();
            for hay in hays {
                for (at, _) in hay.char_indices().chain([(hay.len(), ' ')]) {
                    let want = p
                        .find_at(hay, at)
                        .unwrap()
                        .filter(|m| m.start == at)
                        .map(|m| m.end);
                    assert_eq!(
                        p.match_anchored(hay, at).unwrap(),
                        want,
                        "{src:?} on {hay:?} at {at}"
                    );
                }
            }
        }
        assert!(
            Pattern::compile(
                "(?x) a b # trailing comment",
                false,
                &RegexLimits::default()
            )
            .unwrap()
            .anchored
            .is_none()
        );
        assert_eq!(compile("ab").match_anchored("xab", 2).unwrap(), None);
    }

    fn set(chars: &str) -> u128 {
        chars.bytes().fold(0, |acc, b| acc | (1u128 << b))
    }

    #[test]
    fn first_char_analysis() {
        let fc = |s| first_chars(s, false);
        assert_eq!(fc("abc").unwrap().ascii, set("a"));
        assert_eq!(fc("a|b|\\.x").unwrap().ascii, set("ab."));
        assert_eq!(fc("[A-Za-z_]\\w*").unwrap().ascii, word_ascii() & !DIGITS);
        assert!(!fc("[A-Za-z_]\\w*").unwrap().non_ascii);
        assert!(fc("\\d+").unwrap().non_ascii);
        assert_eq!(fc("[-+*/]+|[;,()]").unwrap().ascii, set("-+*/;,()"));
        assert_eq!(fc(r"[\]\\x]").unwrap().ascii, set("]\\x"));
        assert_eq!(fc(r"\x27a").unwrap().ascii, set("'"));
        assert_eq!(fc(r"[a\x27]").unwrap().ascii, set("a'"));
        assert_eq!(fc("[]a]").unwrap().ascii, set("]a"));
        assert_eq!(fc("[a-]").unwrap().ascii, set("a-"));
        let caret = fc(r"^\s*#").unwrap();
        assert!(caret.at_line_start && caret.ascii == 0);
        let mixed = fc(r"^x|y").unwrap();
        assert!(mixed.at_line_start && mixed.ascii == set("y"));
        // Optional atoms let the next atom start the match.
        assert_eq!(fc("a?b").unwrap().ascii, set("ab"));
        assert_eq!(fc("a*b{0,2}c").unwrap().ascii, set("abc"));
        assert_eq!(fc("[bc]?r#\"").unwrap().ascii, set("bcr"));
        assert_eq!(fc("[rRbB]{0,2}'''").unwrap().ascii, set("rRbB'"));
        assert_eq!(fc("a*?b").unwrap().ascii, set("ab"));
        assert_eq!(fc("a{,2}x").unwrap().ascii, set("ax"));
        // Only optional atoms, or an empty branch: an empty match is no token.
        assert_eq!(fc("a*").unwrap().ascii, set("a"));
        assert_eq!(fc("").unwrap().ascii, 0);
        assert_eq!(fc("a|").unwrap().ascii, set("a"));
        for none in [
            ".x",
            "(a)",
            "a?.",
            "a*(b)",
            "[^a]",
            "[[:alpha:]]",
            "\\bx",
            "(?i)a",
            "a|(?i)b",
            "a(?i)|b",
            "$",
            "\\Dx",
            "[a&&b]",
            "\\pL",
            "\\<a",
            "\\\\(?i)a",
        ] {
            assert!(fc(none).is_none(), "{none:?}");
        }
        assert_eq!(fc("a{2}").unwrap().ascii, set("a"));
        assert!(fc("(?:a)").is_none());
        // Zero-width groups consume nothing.
        assert_eq!(fc(r"(?<=x)y").unwrap().ascii, set("y"));
        assert_eq!(fc(r"(?:^|(?<=\s))#").unwrap().ascii, set("#"));
        assert_eq!(fc(r"(?<=^.{6})[*/].*").unwrap().ascii, set("*/"));
        assert_eq!(fc(r"(?<![a)])(?=b)b").unwrap().ascii, set("b"));
        for none in [r"(?<n>a)b", r"(?:^|x)#", r"(?<=x)?y", r"(?:)a", r"(?<=x"] {
            assert!(fc(none).is_none(), "{none:?}");
        }
        let ci = first_chars("k", true).unwrap();
        assert_eq!(ci.ascii, set("kK"));
        assert!(ci.non_ascii);
        assert_eq!(FirstChars::of_literal("é", false).ascii, 0);
        assert!(FirstChars::of_literal("é", false).non_ascii);
        // KELVIN SIGN and LONG S fold onto ASCII letters: a case-insensitive set with a non-ASCII member admits
        // k, K, s and S (wave-2 review: the prefilter used to reject "key" for the literal "\u{212A}ey").
        let kelvin = FirstChars::of_literal("\u{212A}ey", true);
        assert_eq!(kelvin.ascii & set("kKsS"), set("kKsS"));
        let long_s = first_chars("\u{17F}top", true).unwrap();
        assert_eq!(long_s.ascii & set("sS"), set("sS"));
        let range = first_chars(r"[\x{80}-\x{10FFFF}]+", true).unwrap();
        assert_eq!(range.ascii & set("kKsS"), set("kKsS"));
        // Case-sensitive sets are unchanged.
        assert_eq!(FirstChars::of_literal("\u{212A}ey", false).ascii, 0);
    }
}
