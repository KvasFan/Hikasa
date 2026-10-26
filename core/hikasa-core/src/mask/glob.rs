//! Component patterns: compiler and matcher (§2.2.2; no regex).
//!
//! A component pattern is a sequence of atoms (`*`, `?`, sets and literal chars), compiled from the NFC text of
//! the mask item ([`super::Mask::parse`]); for case-insensitive masks literals and set members are folded with
//! [`simple_lower`]. Names are converted to NFC before matching ([`nfc`] borrows for ASCII and NFC names) and
//! folded char by char, so matching allocates nothing in the common case.
//!
//! Matching is the linear greedy algorithm with a single backtrack point (O(|pattern|·|name|)); the common
//! shapes `literal`, `prefix*`, `*suffix` and `*` take dedicated fast paths with identical results.

use crate::case::{nfc, simple_lower, simple_upper};

use super::MaskError;

/// A name in NFC, prepared once for matching against many patterns (a whole list or filter).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Name<'a> {
    /// The NFC text.
    pub(crate) text: &'a str,
    /// `text.is_ascii()`, computed once.
    pub(crate) ascii: bool,
}

impl<'a> Name<'a> {
    /// Prepares a name that is already in NFC.
    #[inline]
    pub(crate) fn new(text: &'a str) -> Name<'a> {
        Name {
            text,
            ascii: text.is_ascii(),
        }
    }

    /// The first and last byte of a non-empty ASCII name (lower-cased when `insensitive`), which
    /// [`NameBounds::admits`] compares; `None` for empty and non-ASCII names (they are never rejected early).
    #[inline]
    pub(crate) fn edges(self, insensitive: bool) -> Option<(u8, u8)> {
        if !self.ascii {
            return None;
        }
        let bytes = self.text.as_bytes();
        let (&head, &tail) = (bytes.first()?, bytes.last()?);
        Some(if insensitive {
            (head.to_ascii_lowercase(), tail.to_ascii_lowercase())
        } else {
            (head, tail)
        })
    }
}

/// A necessary condition on the item's own name, taken from a pattern's literal atoms: every non-empty ASCII
/// name it matches starts with the byte `first` and ends with the byte `last` (folded for case-insensitive
/// patterns). Lists keep these two bytes per mask in one compact array, so a name is checked against a whole
/// list with two byte comparisons per mask before any mask runs (bench `mask_match`, §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NameBounds {
    /// Required first byte, or [`NameBounds::ANY`].
    first: u8,
    /// Required last byte, or [`NameBounds::ANY`].
    last: u8,
}

impl NameBounds {
    /// No condition (`0xFF` is never an ASCII byte).
    const ANY: u8 = 0xFF;

    /// Admits every name.
    pub(crate) const OPEN: NameBounds = NameBounds {
        first: NameBounds::ANY,
        last: NameBounds::ANY,
    };

    fn new(first: Option<u8>, last: Option<u8>) -> NameBounds {
        NameBounds {
            first: first.unwrap_or(NameBounds::ANY),
            last: last.unwrap_or(NameBounds::ANY),
        }
    }

    /// False only when no name with these [`Name::edges`] can match (`None` edges are always admitted).
    #[inline]
    pub(crate) fn admits(self, edges: Option<(u8, u8)>) -> bool {
        match edges {
            None => true,
            // `|` and `&`, not `||` and `&&`: no branches, so a list's bounds compare in a tight loop.
            Some((head, tail)) => {
                ((self.first == NameBounds::ANY) | (self.first == head))
                    & ((self.last == NameBounds::ANY) | (self.last == tail))
            }
        }
    }
}

/// A bracket expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CharSet {
    /// `[!…]`.
    negated: bool,
    /// Single chars; folded with `simple_lower` when case-insensitive.
    singles: Box<[char]>,
    /// Inclusive ranges, as typed.
    ranges: Box<[(char, char)]>,
}

impl CharSet {
    /// Set membership (§2.2.2): a single item `x` matches `c` when `c == x` (case-insensitive:
    /// `simple_lower(c) == simple_lower(x)`); a range `[lo-hi]` matches when `lo ≤ c ≤ hi` (case-insensitive:
    /// when some `x ∈ {c, simple_lower(c), simple_upper(c)}` is in the range).
    #[inline]
    fn contains(&self, c: char, insensitive: bool) -> bool {
        let hit = if insensitive {
            let lower = simple_lower(c);
            self.singles.contains(&lower)
                || (!self.ranges.is_empty() && {
                    let upper = simple_upper(c);
                    self.ranges.iter().any(|&(lo, hi)| {
                        let range = lo..=hi;
                        range.contains(&c) || range.contains(&lower) || range.contains(&upper)
                    })
                })
        } else {
            self.singles.contains(&c) || self.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
        };
        hit != self.negated
    }
}

/// One atom of a component pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Atom {
    /// `*`: any run of chars, including the empty run.
    Star,
    /// `?`: exactly one char.
    Any,
    /// A literal char (NFC; folded when case-insensitive).
    Lit(char),
    /// `[…]`.
    Set(Box<CharSet>),
}

/// The extension rule of a pattern (§2.2.2), decided by the pattern text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtRule {
    /// No special rule.
    None,
    /// The text ends with a literal `.`: "no extension".
    TrailingDot,
    /// The text ends with `.*`: also matches names without an extension.
    DotStar,
}

/// A compiled atom sequence, specialised for the common shapes.
#[derive(Debug, Clone)]
enum Program {
    /// Only stars: matches every name.
    Any,
    /// Only literals (possibly none: then only the empty name matches).
    Exact(Box<[char]>),
    /// Literals, then stars.
    Prefix(Box<[char]>),
    /// Stars, then literals.
    Suffix(Box<[char]>),
    /// Anything else (consecutive stars collapsed).
    General(Box<[Atom]>),
}

/// A compiled component pattern.
#[derive(Debug, Clone)]
pub(crate) struct Glob {
    /// The atoms as compiled (consecutive stars are *not* collapsed); used by the containment analysis.
    atoms: Box<[Atom]>,
    /// Extension rule.
    rule: ExtRule,
    /// Case-insensitive matching.
    insensitive: bool,
    /// TrailingDot: the atoms without the final `.`; otherwise all atoms.
    primary: Program,
    /// DotStar only: the atoms without the final `.*`.
    secondary: Option<Program>,
    /// The bytes every non-empty ASCII name must start and end with to match (a leading or trailing ASCII
    /// literal, folded when case-insensitive: `README*`, `*.c`): a two-comparison rejection of most names.
    bounds: NameBounds,
}

/// The ASCII byte of a literal atom (already folded for case-insensitive patterns).
fn ascii_literal(atom: Option<&Atom>) -> Option<u8> {
    match atom {
        Some(Atom::Lit(c)) => u8::try_from(*c).ok().filter(u8::is_ascii),
        _ => None,
    }
}

/// Index of the `]` that closes the set opened at `open` (`]` right after `[` or `[!` is literal), searching
/// below `limit`.
pub(crate) fn set_end(chars: &[char], open: usize, limit: usize) -> Option<usize> {
    let mut j = open + 1;
    if chars.get(j) == Some(&'!') {
        j += 1;
    }
    if chars.get(j) == Some(&']') {
        j += 1;
    }
    while j < limit {
        if *chars.get(j)? == ']' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Validates the set `chars[open..=close]` (ranges must not be inverted).
pub(crate) fn check_set(
    chars: &[char],
    open: usize,
    close: usize,
    item: &str,
) -> Result<(), MaskError> {
    parse_set(chars, open, close, item, false).map(|_| ())
}

fn parse_set(
    chars: &[char],
    open: usize,
    close: usize,
    item: &str,
    insensitive: bool,
) -> Result<CharSet, MaskError> {
    let mut j = open + 1;
    let negated = chars.get(j) == Some(&'!');
    if negated {
        j += 1;
    }
    let mut singles = Vec::new();
    let mut ranges = Vec::new();
    while j < close {
        let Some(&c) = chars.get(j) else { break };
        let range_hi = if j + 2 < close && chars.get(j + 1) == Some(&'-') {
            chars.get(j + 2).copied()
        } else {
            None
        };
        match range_hi {
            Some(hi) => {
                if c > hi {
                    return Err(MaskError::InvalidRange {
                        mask: item.to_owned(),
                        at: j,
                        lo: c,
                        hi,
                    });
                }
                ranges.push((c, hi));
                j += 3;
            }
            None => {
                singles.push(if insensitive { simple_lower(c) } else { c });
                j += 1;
            }
        }
    }
    Ok(CharSet {
        negated,
        singles: singles.into_boxed_slice(),
        ranges: ranges.into_boxed_slice(),
    })
}

fn flush_literals(run: &mut String, atoms: &mut Vec<Atom>, insensitive: bool) {
    if run.is_empty() {
        return;
    }
    for c in nfc(run).chars() {
        atoms.push(Atom::Lit(if insensitive { simple_lower(c) } else { c }));
    }
    run.clear();
}

fn literals(atoms: &[Atom]) -> Option<Box<[char]>> {
    atoms
        .iter()
        .map(|atom| {
            if let Atom::Lit(c) = atom {
                Some(*c)
            } else {
                None
            }
        })
        .collect()
}

fn all_stars(atoms: &[Atom]) -> bool {
    !atoms.is_empty() && atoms.iter().all(|atom| matches!(atom, Atom::Star))
}

fn program(atoms: &[Atom]) -> Program {
    let mut collapsed: Vec<Atom> = Vec::with_capacity(atoms.len());
    for atom in atoms {
        if matches!(atom, Atom::Star) && matches!(collapsed.last(), Some(Atom::Star)) {
            continue;
        }
        collapsed.push(atom.clone());
    }
    if all_stars(&collapsed) {
        return Program::Any;
    }
    if let Some(chars) = literals(&collapsed) {
        return Program::Exact(chars);
    }
    if let [head @ .., Atom::Star] = collapsed.as_slice()
        && let Some(chars) = literals(head)
    {
        return Program::Prefix(chars);
    }
    if let [Atom::Star, tail @ ..] = collapsed.as_slice()
        && let Some(chars) = literals(tail)
    {
        return Program::Suffix(chars);
    }
    Program::General(collapsed.into_boxed_slice())
}

/// `has_extension(n)` ⇔ there is a `.` at char index `k` with `0 < k < len(n) − 1` (§2.2.2).
#[inline]
pub(crate) fn has_extension(name: &str) -> bool {
    let bytes = name.as_bytes();
    // '.' is one byte: char index 0 ⇔ byte index 0, last char ⇔ last byte.
    bytes
        .iter()
        .enumerate()
        .any(|(k, &b)| b == b'.' && k > 0 && k + 1 < bytes.len())
}

#[inline]
fn fold_char(c: char, insensitive: bool) -> char {
    if insensitive { simple_lower(c) } else { c }
}

#[inline]
fn fold_byte(b: u8, insensitive: bool) -> char {
    let c = char::from(b);
    if insensitive {
        c.to_ascii_lowercase()
    } else {
        c
    }
}

/// Random access to the chars of a name by byte offset.
trait Text {
    fn byte_len(&self) -> usize;
    /// The char starting at byte `at` and its UTF-8 length.
    fn char_at(&self, at: usize) -> (char, usize);
}

struct Ascii<'a>(&'a [u8]);

impl Text for Ascii<'_> {
    #[inline]
    fn byte_len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn char_at(&self, at: usize) -> (char, usize) {
        (self.0.get(at).map_or('\0', |&b| char::from(b)), 1)
    }
}

struct Utf8<'a>(&'a str);

impl Text for Utf8<'_> {
    #[inline]
    fn byte_len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn char_at(&self, at: usize) -> (char, usize) {
        match self.0.get(at..).and_then(|rest| rest.chars().next()) {
            Some(c) => (c, c.len_utf8()),
            None => ('\0', 1),
        }
    }
}

#[inline]
fn atom_matches(atom: &Atom, c: char, insensitive: bool) -> bool {
    match atom {
        Atom::Any => true,
        Atom::Lit(lit) => fold_char(c, insensitive) == *lit,
        Atom::Set(set) => set.contains(c, insensitive),
        Atom::Star => false,
    }
}

/// The greedy single-backtrack matcher of §2.2.2.
fn greedy<T: Text>(atoms: &[Atom], text: &T, insensitive: bool) -> bool {
    let len = text.byte_len();
    let (mut i, mut j) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while j < len {
        match atoms.get(i) {
            Some(Atom::Star) => {
                star = Some(i);
                mark = j;
                i += 1;
                continue;
            }
            Some(atom) => {
                let (c, width) = text.char_at(j);
                if atom_matches(atom, c, insensitive) {
                    i += 1;
                    j += width;
                    continue;
                }
            }
            None => {}
        }
        match star {
            Some(s) => {
                i = s + 1;
                mark += text.char_at(mark).1;
                j = mark;
            }
            None => return false,
        }
    }
    atoms
        .get(i..)
        .is_some_and(|rest| rest.iter().all(|atom| matches!(atom, Atom::Star)))
}

fn run(program: &Program, name: Name<'_>, insensitive: bool) -> bool {
    let Name { text: name, ascii } = name;
    match program {
        Program::Any => true,
        Program::Exact(lits) => {
            if ascii {
                name.len() == lits.len()
                    && name
                        .bytes()
                        .zip(lits.iter())
                        .all(|(b, &l)| fold_byte(b, insensitive) == l)
            } else {
                let mut chars = name.chars();
                lits.iter()
                    .all(|&l| chars.next().is_some_and(|c| fold_char(c, insensitive) == l))
                    && chars.next().is_none()
            }
        }
        Program::Prefix(lits) => {
            if ascii {
                name.len() >= lits.len()
                    && name
                        .bytes()
                        .zip(lits.iter())
                        .all(|(b, &l)| fold_byte(b, insensitive) == l)
            } else {
                let mut chars = name.chars();
                lits.iter()
                    .all(|&l| chars.next().is_some_and(|c| fold_char(c, insensitive) == l))
            }
        }
        Program::Suffix(lits) => {
            if ascii {
                name.len() >= lits.len()
                    && name
                        .bytes()
                        .rev()
                        .zip(lits.iter().rev())
                        .all(|(b, &l)| fold_byte(b, insensitive) == l)
            } else {
                let mut chars = name.chars().rev();
                lits.iter()
                    .rev()
                    .all(|&l| chars.next().is_some_and(|c| fold_char(c, insensitive) == l))
            }
        }
        Program::General(atoms) => {
            if ascii {
                greedy(atoms, &Ascii(name.as_bytes()), insensitive)
            } else {
                greedy(atoms, &Utf8(name), insensitive)
            }
        }
    }
}

impl Glob {
    /// Compiles the component `chars[start..end]` of the item `item` (whose chars are `chars`). Sets in the
    /// range must be complete (the caller split components outside sets).
    pub(crate) fn compile(
        chars: &[char],
        start: usize,
        end: usize,
        item: &str,
        insensitive: bool,
    ) -> Result<Glob, MaskError> {
        let mut atoms = Vec::with_capacity(end.saturating_sub(start));
        let mut run_text = String::new();
        let mut i = start;
        while i < end {
            let Some(&c) = chars.get(i) else { break };
            match c {
                '*' => {
                    flush_literals(&mut run_text, &mut atoms, insensitive);
                    atoms.push(Atom::Star);
                    i += 1;
                }
                '?' => {
                    flush_literals(&mut run_text, &mut atoms, insensitive);
                    atoms.push(Atom::Any);
                    i += 1;
                }
                '[' => {
                    flush_literals(&mut run_text, &mut atoms, insensitive);
                    let close =
                        set_end(chars, i, end).ok_or_else(|| MaskError::UnclosedBracket {
                            mask: item.to_owned(),
                            at: i,
                        })?;
                    atoms.push(Atom::Set(Box::new(parse_set(
                        chars,
                        i,
                        close,
                        item,
                        insensitive,
                    )?)));
                    i = close + 1;
                }
                _ => {
                    run_text.push(c);
                    i += 1;
                }
            }
        }
        flush_literals(&mut run_text, &mut atoms, insensitive);
        let n = atoms.len();
        let rule = match atoms.as_slice() {
            [.., Atom::Lit('.')] => ExtRule::TrailingDot,
            [.., Atom::Lit('.'), Atom::Star] => ExtRule::DotStar,
            _ => ExtRule::None,
        };
        let (primary, secondary) = match rule {
            ExtRule::None => (program(&atoms), None),
            ExtRule::TrailingDot => (program(atoms.get(..n - 1).unwrap_or(&[])), None),
            ExtRule::DotStar => (
                program(&atoms),
                Some(program(atoms.get(..n - 2).unwrap_or(&[]))),
            ),
        };
        // Every matching name starts with the atoms before the rule's suffix (both alternatives of the dot-star
        // rule do) and, without the dot-star rule, ends with the last atom of the primary alternative.
        let first_basis = match rule {
            ExtRule::None => atoms.as_slice(),
            ExtRule::TrailingDot => atoms.get(..n - 1).unwrap_or(&[]),
            ExtRule::DotStar => atoms.get(..n - 2).unwrap_or(&[]),
        };
        let first = ascii_literal(first_basis.first());
        let last = match rule {
            ExtRule::None => ascii_literal(atoms.last()),
            ExtRule::TrailingDot => ascii_literal(atoms.get(..n - 1).and_then(<[Atom]>::last)),
            ExtRule::DotStar => None,
        };
        let mut glob = Glob {
            atoms: atoms.into_boxed_slice(),
            rule,
            insensitive,
            primary,
            secondary,
            bounds: NameBounds::new(first, last),
        };
        // A universal pattern (`*.*`: the dot-star rule covers the names without a dot) matches every name, so
        // its matcher is skipped (`*` already compiles to `Program::Any`).
        if glob.is_universal() {
            glob.primary = Program::Any;
        }
        Ok(glob)
    }

    /// A necessary condition of [`Glob::matches_nfc`], decided with two byte comparisons: a non-empty ASCII
    /// name must start with the pattern's leading literal and end with its trailing literal (when it has them).
    #[inline]
    pub(crate) fn may_match(&self, name: Name<'_>) -> bool {
        self.bounds.admits(name.edges(self.insensitive))
    }

    /// The condition [`Glob::may_match`] checks.
    pub(crate) fn bounds(&self) -> NameBounds {
        self.bounds
    }

    /// Matches `name`, converting it to NFC first.
    #[inline]
    pub(crate) fn matches(&self, name: &str) -> bool {
        let name = nfc(name);
        self.matches_nfc(Name::new(&name))
    }

    /// Matches a name that is already NFC.
    #[inline]
    pub(crate) fn matches_nfc(&self, name: Name<'_>) -> bool {
        if !self.may_match(name) {
            return false;
        }
        match self.rule {
            ExtRule::None => run(&self.primary, name, self.insensitive),
            ExtRule::TrailingDot => {
                run(&self.primary, name, self.insensitive) && !has_extension(name.text)
            }
            ExtRule::DotStar => {
                run(&self.primary, name, self.insensitive)
                    || (self
                        .secondary
                        .as_ref()
                        .is_some_and(|p| run(p, name, self.insensitive))
                        && !has_extension(name.text))
            }
        }
    }

    /// Same compiled structure (atoms, rule and case mode): then both match exactly the same names.
    pub(crate) fn same_structure(&self, other: &Glob) -> bool {
        self.insensitive == other.insensitive
            && self.rule == other.rule
            && self.atoms == other.atoms
    }

    /// True when the pattern matches every name: only stars, or stars + `.*` (the dot-star rule then covers
    /// names without an extension).
    pub(crate) fn is_universal(&self) -> bool {
        if all_stars(&self.atoms) {
            return true;
        }
        let n = self.atoms.len();
        self.rule == ExtRule::DotStar && n >= 3 && all_stars(self.atoms.get(..n - 2).unwrap_or(&[]))
    }

    /// The atoms every matched name is built from for prefix reasoning: all atoms (no rule), without the
    /// final `.` (trailing dot) or without the final `.*` (dot-star: both alternatives start with them).
    fn prefix_basis(&self) -> &[Atom] {
        let n = self.atoms.len();
        match self.rule {
            ExtRule::None => &self.atoms,
            ExtRule::TrailingDot => self.atoms.get(..n.saturating_sub(1)).unwrap_or(&[]),
            ExtRule::DotStar => self.atoms.get(..n.saturating_sub(2)).unwrap_or(&[]),
        }
    }

    /// Like [`Glob::prefix_basis`] for suffix reasoning; `None` for dot-star patterns (their primary
    /// alternative ends with a star).
    fn suffix_basis(&self) -> Option<&[Atom]> {
        let n = self.atoms.len();
        match self.rule {
            ExtRule::None => Some(&self.atoms),
            ExtRule::TrailingDot => self.atoms.get(..n.saturating_sub(1)),
            ExtRule::DotStar => None,
        }
    }

    /// `Star+ Lit+` with no extension rule: the literal tail.
    fn star_literal_suffix(&self) -> Option<&[Atom]> {
        if self.rule != ExtRule::None {
            return None;
        }
        let stars = self
            .atoms
            .iter()
            .take_while(|atom| matches!(atom, Atom::Star))
            .count();
        let tail = self.atoms.get(stars..)?;
        (stars > 0 && !tail.is_empty() && tail.iter().all(|atom| matches!(atom, Atom::Lit(_))))
            .then_some(tail)
    }

    /// `Lit+ Star+` (any rule: a dot-star rule only widens the language): the literal head.
    fn literal_star_prefix(&self) -> Option<&[Atom]> {
        let lits = self
            .atoms
            .iter()
            .take_while(|atom| matches!(atom, Atom::Lit(_)))
            .count();
        let rest = self.atoms.get(lits..)?;
        (lits > 0 && !rest.is_empty() && rest.iter().all(|atom| matches!(atom, Atom::Star)))
            .then(|| self.atoms.get(..lits).unwrap_or(&[]))
    }

    /// Sound containment of name languages (§2.12.3): `true` only if every name matched by `self` is matched
    /// by `other`. Cases: (a) `other` is universal (`*`, `*.*`); (b)/(e) same structure; (c) `other` is
    /// `*` + literal suffix S and every name of `self` ends with S; (d) `other` is a literal prefix P + `*`
    /// and every name of `self` starts with P. (c)–(e) require the same case mode.
    pub(crate) fn is_covered_by(&self, other: &Glob) -> bool {
        if other.is_universal() {
            return true;
        }
        if self.insensitive != other.insensitive {
            return false;
        }
        if self.same_structure(other) {
            return true;
        }
        if let (Some(suffix), Some(basis)) = (other.star_literal_suffix(), self.suffix_basis())
            && basis.ends_with(suffix)
        {
            return true;
        }
        other
            .literal_star_prefix()
            .is_some_and(|prefix| self.prefix_basis().starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(text: &str, insensitive: bool) -> Glob {
        let chars: Vec<char> = text.chars().collect();
        Glob::compile(&chars, 0, chars.len(), text, insensitive).unwrap()
    }

    #[test]
    fn programs_are_specialised() {
        assert!(matches!(glob("*", false).primary, Program::Any));
        assert!(matches!(glob("**", false).primary, Program::Any));
        assert!(matches!(glob("abc", false).primary, Program::Exact(_)));
        assert!(matches!(glob("ab*", false).primary, Program::Prefix(_)));
        assert!(matches!(glob("*.c", false).primary, Program::Suffix(_)));
        assert!(matches!(glob("a*b", false).primary, Program::General(_)));
        let dot = glob("*.", false);
        assert_eq!(dot.rule, ExtRule::TrailingDot);
        assert!(matches!(dot.primary, Program::Any));
        let dot_star = glob("abc.*", false);
        assert_eq!(dot_star.rule, ExtRule::DotStar);
        assert!(matches!(dot_star.primary, Program::Prefix(_)));
        assert!(matches!(dot_star.secondary, Some(Program::Exact(_))));
        assert_eq!(glob("a.**", false).rule, ExtRule::None);
        assert_eq!(glob("a[.]", false).rule, ExtRule::None);
    }

    #[test]
    fn fast_paths_agree_with_the_general_matcher() {
        let names = [
            "",
            "a",
            "abc",
            "ABC",
            "x.c",
            "X.C",
            "abc.c",
            "é.c",
            "É.C",
            "cafe\u{301}",
            "k",
            "\u{212A}",
        ];
        for pattern in ["*", "abc", "ab*", "*.c", "*c", "é*", "*É.c", "k"] {
            for insensitive in [false, true] {
                let compiled = glob(pattern, insensitive);
                let general = Program::General(compiled.atoms.clone());
                for name in names {
                    let name = nfc(name);
                    let prepared = Name::new(&name);
                    assert_eq!(
                        run(&compiled.primary, prepared, insensitive),
                        run(&general, prepared, insensitive),
                        "{pattern:?} vs {name:?} (insensitive {insensitive})"
                    );
                }
            }
        }
    }

    #[test]
    fn literal_bounds_are_necessary_conditions() {
        let bounds = |text: &str, insensitive: bool| glob(text, insensitive).bounds();
        assert_eq!(bounds("*.c", false), NameBounds::new(None, Some(b'c')));
        assert_eq!(bounds("*.C", true), NameBounds::new(None, Some(b'c')));
        assert_eq!(bounds("README*", false), NameBounds::new(Some(b'R'), None));
        assert_eq!(
            bounds("abc.", false),
            NameBounds::new(Some(b'a'), Some(b'c'))
        );
        assert_eq!(bounds("abc.*", false), NameBounds::new(Some(b'a'), None));
        assert_eq!(bounds(".*", false), NameBounds::OPEN);
        assert_eq!(bounds("[ab]*?", false), NameBounds::OPEN);
        assert_eq!(bounds("é*", false), NameBounds::OPEN);
        assert_eq!(Name::new("Ab.C").edges(true), Some((b'a', b'c')));
        assert_eq!(Name::new("Ab.C").edges(false), Some((b'A', b'C')));
        assert_eq!(Name::new("").edges(true), None);
        assert_eq!(Name::new("é.c").edges(true), None);
        assert!(NameBounds::new(Some(b'a'), None).admits(None));
        assert!(!NameBounds::new(Some(b'a'), None).admits(Some((b'b', b'a'))));
        assert!(NameBounds::OPEN.admits(Some((0, 0x7F))));
        let g = glob("*.c", true);
        assert!(g.may_match(Name::new("X.C")));
        assert!(!g.may_match(Name::new("x.h")));
        assert!(g.may_match(Name::new("")), "empty names are not filtered");
        assert!(
            g.may_match(Name::new("x.ç")),
            "non-ASCII names are not filtered"
        );
        let dot_star = glob(".*", false);
        assert!(
            dot_star.matches_nfc(Name::new("")),
            "`.*` matches the empty name"
        );
        let universal = glob("**.*", false);
        assert!(matches!(universal.primary, Program::Any));
        for name in ["", ".", "a", "a.", ".a", "a.b.c", "é"] {
            assert!(universal.matches_nfc(Name::new(name)), "{name:?}");
        }
    }

    #[test]
    fn extensions() {
        assert!(!has_extension("Makefile"));
        assert!(!has_extension(".bashrc"));
        assert!(!has_extension("a."));
        assert!(!has_extension("."));
        assert!(!has_extension(".."));
        assert!(has_extension("a.b"));
        assert!(has_extension("a.tar.gz"));
        assert!(has_extension(".a.b"));
    }

    #[test]
    fn containment_cases() {
        let covered = |i: &str, j: &str| glob(i, true).is_covered_by(&glob(j, true));
        assert!(covered("*.c", "*"));
        assert!(covered("Makefile", "*.*"));
        assert!(covered("test*.c", "*.c"), "(c)");
        assert!(covered("abc*", "a*"), "(d)");
        assert!(covered("abc.", "abc*"), "(d) with a trailing dot");
        assert!(covered("x.c", "X.C"), "(b)/(e) after folding");
        assert!(
            !covered("abc.", "abc.**"),
            "abc. matches 'abc', which abc.** does not"
        );
        assert!(!covered("abc.*", "abc.**"));
        assert!(!covered("*.h", "*.c"));
        assert!(!covered("a*", "*.c"));
        assert!(
            !glob("x.c", false).is_covered_by(&glob("X.C", false)),
            "case-sensitive"
        );
        assert!(
            !glob("x.c", true).is_covered_by(&glob("x.c", false)),
            "different case modes"
        );
    }
}
