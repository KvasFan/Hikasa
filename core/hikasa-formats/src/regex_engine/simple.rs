//! An exact accelerator for simple patterns (§2.10.5: fast paths must not change results).
//!
//! Most grammar regexes are short sequences of character classes (`[A-Za-z_]\w*`, `[-+*/=]+|[;,()]`, number
//! literals). For these, asking "does a match start exactly at `p`, and where does it end?" through the
//! general engine costs a cache checkout plus a forward and a reverse automaton scan per token. This module
//! answers the same question with a tiny backtracking matcher over ASCII.
//!
//! Exactness: the matcher implements leftmost-first priority (alternatives in order, greedy quantifiers longest
//! first, optional groups taken first), which is the priority of the `regex` crate. It only ever decides on
//! ASCII input: the moment it would have to classify a non-ASCII char (where Unicode classes and case folding
//! need full tables), or when its step budget runs out, it gives up and the caller asks the real engine. The
//! supported syntax is a strict subset; anything else (look-around, word boundaries, lazy quantifiers, inline
//! flags, Unicode classes, repeated groups, non-ASCII literals) is not compiled here at all.

/// Steps (node visits) before the matcher gives up and defers to the real engine (T6: a count, not a time).
const STEP_BUDGET: u32 = 4096;
/// Bytes past the start position the matcher may examine before it defers to the real engine. Without this
/// bound, a failing attempt could scan a long run from every position of the run (`[ \t]+$` inside a long
/// run of blanks, `\$\{[^}]*\}` without a closer): quadratic time on one line (§2.10.5).
const SCAN_BUDGET: usize = 512;
/// Largest node count accepted.
const MAX_NODES: usize = 256;

#[derive(Debug, Clone)]
enum Node {
    /// A character class over ASCII with a repetition `min..=max`.
    Set { set: u128, min: u32, max: u32 },
    /// A group of alternatives, optional when `optional` (the only group quantifier supported).
    Group {
        alts: Vec<Vec<Node>>,
        optional: bool,
    },
    /// `^` / `\A`: the start of the haystack.
    Start,
    /// `$` / `\z`: the end of the haystack.
    End,
}

/// A compiled simple pattern.
#[derive(Debug, Clone)]
pub(crate) struct Simple {
    alts: Vec<Vec<Node>>,
    /// Each alternative as a straight-line program, when it is *linear* (see [`linear`]).
    linear: Vec<Option<Linear>>,
}

/// A branch made only of character sets (plus `^` first and `$` last) in which greedy repetition never needs to
/// backtrack: every variable repetition is disjoint from everything that can follow it up to the next mandatory
/// set. Giving back a char of such a repetition would leave a char that no following node accepts, so the
/// greedy count is the only one that can succeed (and when every following node is optional it succeeds
/// first). Matching is then one forward pass.
#[derive(Debug, Clone)]
struct Linear {
    start: bool,
    end: bool,
    sets: Vec<(u128, u32, u32)>,
}

fn linear(alt: &[Node]) -> Option<Linear> {
    let mut start = false;
    let mut end = false;
    let mut sets = Vec::new();
    for (k, node) in alt.iter().enumerate() {
        match node {
            Node::Start if k == 0 => start = true,
            Node::End if k + 1 == alt.len() => end = true,
            Node::Set { set, min, max } => sets.push((*set, *min, *max)),
            _ => return None,
        }
    }
    for (k, &(set, min, max)) in sets.iter().enumerate() {
        if min == max {
            continue;
        }
        let mut follow = 0u128;
        for &(next, next_min, _) in &sets[k + 1..] {
            follow |= next;
            if next_min > 0 {
                break;
            }
        }
        if follow & set != 0 {
            return None;
        }
    }
    Some(Linear { start, end, sets })
}

impl Linear {
    /// The match end at `p` and the number of bytes examined.
    #[inline]
    fn match_at(&self, bytes: &[u8], p: usize) -> Result<(Option<usize>, usize), Undecided> {
        if self.start && p != 0 {
            return Ok((None, 0));
        }
        let limit = p.saturating_add(SCAN_BUDGET);
        let mut q = p;
        for &(set, min, max) in &self.sets {
            let mut n = 0u32;
            while n < max {
                let Some(&b) = bytes.get(q) else {
                    break;
                };
                if b >= 128 || q >= limit {
                    return Err(Undecided);
                }
                if set & (1u128 << b) == 0 {
                    break;
                }
                n += 1;
                q += 1;
            }
            if n < min {
                return Ok((None, q + 1 - p));
            }
        }
        let examined = q + 1 - p;
        if self.end && q != bytes.len() {
            return Ok((None, examined));
        }
        Ok((Some(q), examined))
    }
}

/// The matcher could not decide (non-ASCII input examined, or the step budget ran out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Undecided;

struct Cont<'a> {
    nodes: &'a [Node],
    next: Option<&'a Cont<'a>>,
}

struct Run<'h> {
    bytes: &'h [u8],
    steps: u32,
    /// Bytes at or after this index are not examined (the scan budget).
    limit: usize,
    /// One past the furthest index examined.
    far: usize,
}

impl Run<'_> {
    fn seq(
        &mut self,
        nodes: &[Node],
        pos: usize,
        cont: Option<&Cont<'_>>,
    ) -> Result<Option<usize>, Undecided> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            return Err(Undecided);
        }
        let Some((first, rest)) = nodes.split_first() else {
            return match cont {
                Some(c) => self.seq(c.nodes, pos, c.next),
                None => Ok(Some(pos)),
            };
        };
        match first {
            Node::Start => {
                if pos == 0 {
                    self.seq(rest, pos, cont)
                } else {
                    Ok(None)
                }
            }
            Node::End => {
                if pos == self.bytes.len() {
                    self.seq(rest, pos, cont)
                } else {
                    Ok(None)
                }
            }
            Node::Set { set, min, max } => {
                let mut n = 0u32;
                let mut q = pos;
                while n < *max {
                    let Some(&b) = self.bytes.get(q) else {
                        break;
                    };
                    if b >= 128 || q >= self.limit {
                        return Err(Undecided);
                    }
                    if set & (1u128 << b) == 0 {
                        break;
                    }
                    n += 1;
                    q += 1;
                }
                self.far = self.far.max(q + 1);
                if n < *min {
                    return Ok(None);
                }
                for k in (*min..=n).rev() {
                    if let Some(end) = self.seq(rest, pos + k as usize, cont)? {
                        return Ok(Some(end));
                    }
                }
                Ok(None)
            }
            Node::Group { alts, optional } => {
                let after = Cont {
                    nodes: rest,
                    next: cont,
                };
                for alt in alts {
                    if let Some(end) = self.seq(alt, pos, Some(&after))? {
                        return Ok(Some(end));
                    }
                }
                if *optional {
                    self.seq(rest, pos, cont)
                } else {
                    Ok(None)
                }
            }
        }
    }
}

impl Simple {
    /// Compiles `source` if it is in the supported subset (with the builder's case-insensitive flag).
    pub(crate) fn compile(source: &str, case_insensitive: bool) -> Option<Simple> {
        let chars: Vec<char> = source.chars().collect();
        let mut parser = Parser {
            chars: &chars,
            at: 0,
            ci: case_insensitive,
            nodes: 0,
        };
        let alts = parser.alternation()?;
        if parser.at != chars.len() || parser.nodes > MAX_NODES {
            return None;
        }
        let linear = alts.iter().map(|a| linear(a)).collect();
        Some(Simple { alts, linear })
    }

    /// The end of the leftmost-first match starting exactly at `p` (`None`: no match starts there; the end
    /// may equal `p` for an empty match) and the number of bytes examined from `p` on, or `Undecided`.
    #[inline]
    pub(crate) fn match_at(
        &self,
        haystack: &str,
        p: usize,
    ) -> Result<(Option<usize>, usize), Undecided> {
        let bytes = haystack.as_bytes();
        let mut run = Run {
            bytes,
            steps: 0,
            limit: p.saturating_add(SCAN_BUDGET),
            far: p,
        };
        let mut examined = 0usize;
        for (alt, linear) in self.alts.iter().zip(&self.linear) {
            let found = match linear {
                Some(linear) => {
                    let (found, seen) = linear.match_at(bytes, p)?;
                    examined = examined.max(seen);
                    found
                }
                None => {
                    let found = run.seq(alt, p, None)?;
                    examined = examined.max(run.far - p);
                    found
                }
            };
            if let Some(end) = found {
                return Ok((Some(end), examined));
            }
        }
        Ok((None, examined))
    }

    /// The general backtracking matcher for every alternative (the reference for [`Linear`] in tests).
    #[cfg(test)]
    fn backtrack(&self, haystack: &str, p: usize) -> Result<Option<usize>, Undecided> {
        let mut run = Run {
            bytes: haystack.as_bytes(),
            steps: 0,
            limit: p.saturating_add(SCAN_BUDGET),
            far: p,
        };
        for alt in &self.alts {
            if let Some(end) = run.seq(alt, p, None)? {
                return Ok(Some(end));
            }
        }
        Ok(None)
    }
}

struct Parser<'c> {
    chars: &'c [char],
    at: usize,
    ci: bool,
    nodes: usize,
}

fn bit(c: char) -> u128 {
    1u128 << (c as u32)
}

fn range(lo: u8, hi: u8) -> u128 {
    (lo..=hi).fold(0, |acc, b| acc | (1u128 << b))
}

const DIGIT: u128 = 0x03FF << b'0';
fn word() -> u128 {
    DIGIT | range(b'a', b'z') | range(b'A', b'Z') | (1u128 << b'_')
}
const SPACE: u128 = (1u128 << b'\t')
    | (1u128 << b'\n')
    | (1u128 << 0x0B)
    | (1u128 << 0x0C)
    | (1u128 << b'\r')
    | (1u128 << b' ');
const ASCII: u128 = u128::MAX;

fn fold(set: u128) -> u128 {
    let mut out = set;
    for b in b'a'..=b'z' {
        let lower = 1u128 << b;
        let upper = 1u128 << (b - 32);
        if set & (lower | upper) != 0 {
            out |= lower | upper;
        }
    }
    out
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(c)
    }

    fn alternation(&mut self) -> Option<Vec<Vec<Node>>> {
        let mut alts = vec![self.sequence()?];
        while self.peek() == Some('|') {
            self.at += 1;
            alts.push(self.sequence()?);
        }
        Some(alts)
    }

    fn sequence(&mut self) -> Option<Vec<Node>> {
        let mut seq = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            self.nodes += 1;
            let node = match c {
                '(' => {
                    self.at += 1;
                    if self.peek() == Some('?') {
                        self.at += 1;
                        if self.next()? != ':' {
                            return None;
                        }
                    }
                    let alts = self.alternation()?;
                    if self.next()? != ')' {
                        return None;
                    }
                    let optional = match self.peek() {
                        Some('?') => {
                            self.at += 1;
                            true
                        }
                        Some('*' | '+' | '{') => return None,
                        _ => false,
                    };
                    if optional && self.peek() == Some('?') {
                        return None;
                    }
                    Node::Group { alts, optional }
                }
                '^' => {
                    self.at += 1;
                    Node::Start
                }
                '$' => {
                    self.at += 1;
                    Node::End
                }
                '*' | '+' | '?' | '{' | '}' | ']' => return None,
                _ => {
                    let set = self.atom()?;
                    let set = if self.ci { fold(set) } else { set };
                    let (min, max) = self.quantifier()?;
                    Node::Set { set, min, max }
                }
            };
            seq.push(node);
        }
        Some(seq)
    }

    /// One class atom (literal, escape, `.` or bracket class) as an ASCII set.
    fn atom(&mut self) -> Option<u128> {
        let c = self.next()?;
        match c {
            '.' => Some(ASCII & !bit('\n')),
            '[' => self.class(),
            '\\' => {
                let e = self.next()?;
                self.escape(e)
            }
            c if c.is_ascii() => Some(bit(c)),
            _ => None,
        }
    }

    /// An escape after `\` (outside or inside classes): classes, control escapes, hex, ASCII punctuation.
    fn escape(&mut self, e: char) -> Option<u128> {
        match e {
            'd' => Some(DIGIT),
            'w' => Some(word()),
            's' => Some(SPACE),
            'D' => Some(ASCII & !DIGIT),
            'W' => Some(ASCII & !word()),
            'S' => Some(ASCII & !SPACE),
            't' => Some(bit('\t')),
            'n' => Some(bit('\n')),
            'r' => Some(bit('\r')),
            'f' => Some(bit('\x0C')),
            'v' => Some(bit('\x0B')),
            'a' => Some(bit('\x07')),
            'x' => {
                let c = self.hex()?;
                c.is_ascii().then(|| bit(c))
            }
            '<' | '>' => None,
            c if c.is_ascii_punctuation() => Some(bit(c)),
            _ => None,
        }
    }

    fn hex(&mut self) -> Option<char> {
        let mut digits = String::new();
        if self.peek() == Some('{') {
            self.at += 1;
            loop {
                let c = self.next()?;
                if c == '}' {
                    break;
                }
                digits.push(c);
            }
        } else {
            digits.push(self.next()?);
            digits.push(self.next()?);
        }
        char::from_u32(u32::from_str_radix(&digits, 16).ok()?)
    }

    /// A bracket class after `[`.
    fn class(&mut self) -> Option<u128> {
        let negated = self.peek() == Some('^');
        if negated {
            self.at += 1;
        }
        let mut set = 0u128;
        let mut first = true;
        loop {
            let c = self.next()?;
            match c {
                ']' if !first => break,
                '[' => return None,
                '&' | '~' | '-' if self.peek() == Some(c) && !first => return None,
                _ => {}
            }
            first = false;
            let lo_set = if c == '\\' {
                let e = self.next()?;
                self.escape(e)?
            } else if c.is_ascii() {
                bit(c)
            } else {
                return None;
            };
            // A range `lo-hi` (a `-` before `]` is literal).
            if self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|&n| n != ']') {
                if lo_set.count_ones() != 1 {
                    return None;
                }
                let lo = lo_set.trailing_zeros() as u8;
                self.at += 1;
                let h = self.next()?;
                let hi = match h {
                    '\\' => {
                        let e = self.next()?;
                        let s = self.escape(e)?;
                        if s.count_ones() != 1 {
                            return None;
                        }
                        s.trailing_zeros() as u8
                    }
                    '[' => return None,
                    h if h.is_ascii() => h as u8,
                    _ => return None,
                };
                if hi < lo {
                    return None;
                }
                set |= range(lo, hi);
            } else {
                set |= lo_set;
            }
        }
        if self.ci {
            set = fold(set);
        }
        Some(if negated { !set } else { set })
    }

    /// A quantifier after a class atom: none, `?`, `*`, `+`, `{n}`, `{n,}`, `{n,m}` (greedy only).
    fn quantifier(&mut self) -> Option<(u32, u32)> {
        let q = match self.peek() {
            Some('?') => (0, 1),
            Some('*') => (0, u32::MAX),
            Some('+') => (1, u32::MAX),
            Some('{') => {
                self.at += 1;
                let mut text = String::new();
                loop {
                    let c = self.next()?;
                    if c == '}' {
                        break;
                    }
                    text.push(c);
                }
                let (min, max) = match text.split_once(',') {
                    None => {
                        let n = text.trim().parse().ok()?;
                        (n, n)
                    }
                    Some((a, b)) => {
                        let min = a.trim().parse().ok()?;
                        let max = if b.trim().is_empty() {
                            u32::MAX
                        } else {
                            b.trim().parse().ok()?
                        };
                        (min, max)
                    }
                };
                if max < min || min > 1000 || (max != u32::MAX && max > 1000) {
                    return None;
                }
                if self.peek() == Some('?') {
                    return None;
                }
                return Some((min, max));
            }
            _ => return Some((1, 1)),
        };
        self.at += 1;
        if self.peek() == Some('?') {
            return None;
        }
        Some(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(src: &str, hay: &str, p: usize) -> Result<Option<usize>, Undecided> {
        Simple::compile(src, false)
            .unwrap()
            .match_at(hay, p)
            .map(|(end, _)| end)
    }

    #[test]
    fn supported_subset() {
        for ok in [
            r"[A-Za-z_]\w*",
            r"[-+*/%=&|^!~<>?:.]+|[;,()\[\]{}@#$\\]",
            r"0[xX][0-9A-Fa-f\x27]+[uUlL]*|\d[\d\x27]*(?:\.[\d\x27]*)?(?:[eE][+-]?\d+)?[uUlLfFdDmM]*",
            r"^\s*#\s*[A-Za-z_]\w*",
            r"[ \t]+$",
            r"(a|bc)?d{2,3}",
            r"[^\]\\]",
        ] {
            assert!(Simple::compile(ok, false).is_some(), "{ok}");
        }
        for unsupported in [
            r"\bx",
            r"(?i)x",
            r"a*?",
            r"(ab)*",
            r"(ab)+",
            r"\p{L}",
            "é",
            r"(?=x)",
            r"(?<=x)y",
            r"[[:alpha:]]",
            r"a{2}?",
            r"\<x",
            r"[a&&b]",
            r"\1",
        ] {
            assert!(
                Simple::compile(unsupported, false).is_none(),
                "{unsupported}"
            );
        }
    }

    #[test]
    fn leftmost_first_priority() {
        assert_eq!(m(r"[A-Za-z_]\w*", "foo_1 x", 0), Ok(Some(5)));
        assert_eq!(m(r"[A-Za-z_]\w*", "1foo", 0), Ok(None));
        assert_eq!(m("a|ab", "abc", 0), Ok(Some(1)));
        assert_eq!(m("ab|a", "abc", 0), Ok(Some(2)));
        assert_eq!(m(r"\d+(?:\.\d+)?", "12.5x", 0), Ok(Some(4)));
        assert_eq!(m(r"\d+(?:\.\d+)?", "12.x", 0), Ok(Some(2)));
        assert_eq!(m("a*", "bbb", 0), Ok(Some(0)));
        assert_eq!(m("a*ab", "aaab", 0), Ok(Some(4)));
        assert_eq!(m("^a", "aa", 1), Ok(None));
        assert_eq!(m("a$", "aa", 1), Ok(Some(2)));
        assert_eq!(m("a$", "aa", 0), Ok(None));
        assert_eq!(m("[^a]", "ba", 0), Ok(Some(1)));
    }

    #[test]
    fn non_ascii_input_is_undecided() {
        assert_eq!(m(r"\w+", "aé", 0), Err(Undecided));
        assert_eq!(m(r"\w+", "éa", 0), Err(Undecided));
        // A char that is never examined does not matter.
        assert_eq!(m(r"ab", "abé", 0), Ok(Some(2)));
        assert_eq!(m(r"\d", "é", 2), Ok(None));
    }

    #[test]
    fn case_insensitive_sets() {
        let s = Simple::compile("[a-c]x", true).unwrap();
        assert_eq!(s.match_at("BX", 0), Ok((Some(2), 3)));
        let neg = Simple::compile("[^a]", true).unwrap();
        assert_eq!(neg.match_at("A", 0), Ok((None, 1)));
    }

    /// A scan never examines more than `SCAN_BUDGET` bytes: a long run defers to the regex engine, so that a
    /// failing attempt from every position of a long run cannot cost quadratic time (§2.10.5).
    #[test]
    fn scan_budget_defers_long_runs() {
        let run = format!("x{}y", " ".repeat(SCAN_BUDGET * 4));
        assert_eq!(m(r"[ \t]+$", &run, 1), Err(Undecided));
        assert_eq!(
            m(
                r"\$\{[^}]*\}",
                &format!("${{{}", "a".repeat(SCAN_BUDGET * 2)),
                0
            ),
            Err(Undecided)
        );
        // Short scans report what they examined.
        let s = Simple::compile(r"[ \t]+$", false).unwrap();
        assert_eq!(s.match_at("x   y", 1), Ok((None, 4)));
        assert_eq!(s.match_at("x   ", 1), Ok((Some(4), 4)));
        let backtracking = Simple::compile("(?:a|ab)c", false).unwrap();
        assert!(backtracking.linear.iter().all(Option::is_none));
        assert_eq!(backtracking.match_at("abx", 0), Ok((None, 3)));
        let long = format!("{}c", "a".repeat(SCAN_BUDGET * 2));
        assert_eq!(
            Simple::compile("a+c", false).unwrap().match_at(&long, 0),
            Err(Undecided)
        );
        assert_eq!(
            Simple::compile("(?:a)?a+c", false)
                .unwrap()
                .match_at(&long, 0),
            Err(Undecided)
        );
    }

    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;

    /// 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are kept with the integration
    /// tests' regressions.
    fn config() -> ProptestConfig {
        ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/proptest-regressions/simple.txt",
            ))),
            ..ProptestConfig::default()
        }
    }

    fn atom() -> impl Strategy<Value = String> {
        let base = prop::sample::select(vec![
            "a", "b", "x", "-", " ", "1", "[ab]", "[^a]", "[a-c]", r"\d", r"\w", r"\s", r"\W", ".",
            r"\.", r"\-", "[-x]", r"[\]a]", r"\x41",
        ]);
        let quant =
            prop::sample::select(vec!["", "", "?", "*", "+", "{1,2}", "{2}", "{0,3}", "{2,}"]);
        (base, quant).prop_map(|(b, q)| format!("{b}{q}"))
    }

    fn seq(depth: u32) -> BoxedStrategy<String> {
        let leaf = prop::collection::vec(atom(), 0..4).prop_map(|v| v.concat());
        if depth == 0 {
            return leaf.boxed();
        }
        let group =
            (prop::collection::vec(seq(depth - 1), 1..3), any::<bool>()).prop_map(|(alts, opt)| {
                format!("(?:{}){}", alts.join("|"), if opt { "?" } else { "" })
            });
        let anchor = prop::sample::select(vec!["", "^", "$"]);
        (
            anchor,
            leaf,
            prop::option::of(group),
            prop::collection::vec(atom(), 0..2),
        )
            .prop_map(|(a, l, g, tail)| format!("{a}{l}{}{}", g.unwrap_or_default(), tail.concat()))
            .boxed()
    }

    fn pattern() -> impl Strategy<Value = String> {
        prop::collection::vec(seq(2), 1..3).prop_map(|alts| alts.join("|"))
    }

    fn haystack() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec![
                'a', 'b', 'c', 'x', '-', '.', ' ', '1', 'é', 'A', '\t', ']', 'K',
            ]),
            0..12,
        )
        .prop_map(|c| c.into_iter().collect())
    }

    proptest! {
        #![proptest_config(config())]

        /// The accelerator agrees with the `regex` crate whenever it decides (exactness, §2.10.5).
        #[test]
        fn agrees_with_the_regex_crate(src in pattern(), hay in haystack(), ci in any::<bool>()) {
            let Ok(re) = regex::RegexBuilder::new(&src).case_insensitive(ci).build() else {
                return Ok(());
            };
            let Some(simple) = Simple::compile(&src, ci) else {
                return Ok(());
            };
            for (p, _) in hay.char_indices().chain([(hay.len(), ' ')]) {
                let want = re.find_at(&hay, p).filter(|m| m.start() == p).map(|m| m.end());
                if let Ok((got, examined)) = simple.match_at(&hay, p) {
                    prop_assert_eq!(got, want, "pattern {:?} haystack {:?} at {}", src, hay, p);
                    prop_assert!(examined <= hay.len() + 1 - p, "examined {} at {} of {:?}", examined, p, hay);
                }
                if let Ok(got) = simple.backtrack(&hay, p) {
                    prop_assert_eq!(got, want, "backtracking: pattern {:?} haystack {:?} at {}", src, hay, p);
                }
            }
        }

        /// The prefilter's first-char analysis is conservative: every non-empty match that starts at `p` starts
        /// with a char the analysis admits (§2.10.5).
        #[test]
        fn first_chars_are_conservative(src in pattern(), hay in haystack(), ci in any::<bool>()) {
            let Ok(re) = regex::RegexBuilder::new(&src).case_insensitive(ci).build() else {
                return Ok(());
            };
            let Some(fc) = super::super::first_chars(&src, ci) else {
                return Ok(());
            };
            for (p, c) in hay.char_indices() {
                let Some(m) = re.find_at(&hay, p).filter(|m| m.start() == p && m.end() > p) else {
                    continue;
                };
                let admitted = (p == 0 && fc.at_line_start)
                    || if c.is_ascii() { fc.ascii & (1u128 << (c as u32)) != 0 } else { fc.non_ascii };
                prop_assert!(admitted, "pattern {:?} matches {:?} at {}", src, &hay[m.range()], p);
            }
        }
    }

    proptest! {
        #![proptest_config(config())]

        /// The first-char analysis also skips zero-width prefixes correctly (fancy-regex patterns).
        #[test]
        fn first_chars_with_zero_width_prefixes(
            prefix in prop::sample::select(vec![
                "(?<=a)", "(?<!b)", r"(?:^|(?<=\s))", "(?=x)", "(?<=^.{2})", "(?:^|(?<=[ab]))", "(?!a)",
            ]),
            src in seq(1),
            hay in haystack(),
        ) {
            let src = format!("{prefix}{src}");
            let Ok(pattern) = super::super::Pattern::compile(&src, false, &super::super::RegexLimits::default())
            else {
                return Ok(());
            };
            let Some(fc) = super::super::first_chars(&src, false) else {
                return Ok(());
            };
            for (p, c) in hay.char_indices() {
                let Ok(Some(m)) = pattern.find_at(&hay, p) else {
                    continue;
                };
                if m.start != p || m.end == p {
                    continue;
                }
                let admitted = (p == 0 && fc.at_line_start)
                    || if c.is_ascii() { fc.ascii & (1u128 << (c as u32)) != 0 } else { fc.non_ascii };
                prop_assert!(admitted, "pattern {:?} matches {:?} at {}", src, &hay[m], p);
            }
        }
    }

    #[test]
    fn linear_programs() {
        let is_linear = |src: &str| {
            Simple::compile(src, false)
                .unwrap()
                .linear
                .iter()
                .all(Option::is_some)
        };
        assert!(is_linear(r"[A-Za-z_]\w*"));
        assert!(is_linear(r"[-+*/%=&|^!~<>?:.]+|[;,()\[\]{}@#$\\]"));
        assert!(is_linear(r"^\s*#\s*[A-Za-z_]\w*"));
        assert!(is_linear(r"0[xX][0-9A-Fa-f\x27]+[uUlL]*"));
        assert!(is_linear(r"[ \t]+$"));
        assert!(!is_linear(r"a*ab"));
        assert!(!is_linear(r"\d+(?:\.\d+)?"));
        assert!(!is_linear(r"[a-z]*[aeiou]"));
        assert_eq!(m(r"[ \t]+$", "  x", 0), Ok(None));
        assert_eq!(m(r"[ \t]+$", "x  ", 1), Ok(Some(3)));
        assert_eq!(m(r"a*b?", "aac", 0), Ok(Some(2)));
    }

    #[test]
    fn step_budget() {
        let s = Simple::compile(r"\d*\d*\d*\d*x", false).unwrap();
        assert_eq!(s.match_at(&"1".repeat(200), 0), Err(Undecided));
    }
}
