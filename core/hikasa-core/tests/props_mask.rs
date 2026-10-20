//! Mask properties P-MASK-1..4 (A1.3: 4096 cases unless `PROPTEST_CASES` overrides), plus soundness of the
//! containment test (§2.12.3) and of `may_match_below` (D-09).
//!
//! P-MASK-3 is checked for every name whose case fold keeps its meaning under the normative rules of §2.2.2
//! (names are compared as NFC chars with `simple_lower`; see `fold_is_regular`); irregular names match like
//! their NFC form, and `tests/mask_table.rs` pins their behaviour.
//!
//! The oracle of P-MASK-1 is an independent implementation: its own parser, `regex` for component matching
//! (the extension rules are applied as boolean combinations of regex matches) and a recursive backtracking
//! path matcher.

mod common;

use hikasa_core::case::{fold, nfc, simple_lower, simple_upper};
use hikasa_core::mask::{
    CaseSensitivity, Mask, MaskError, MaskList, MaskOptions, MaskTarget, Polarity,
};
use hikasa_core::path::RelPath;
use proptest::prelude::*;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OracleError {
    Unclosed,
    Range,
    Empty,
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Plain,
    TrailingDot,
    DotStar,
}

#[derive(Debug)]
struct OraclePattern {
    full: Regex,
    shortened: Option<Regex>,
    rule: Rule,
}

#[derive(Debug)]
enum OracleComponent {
    AnyDepth,
    Pattern(OraclePattern),
}

#[derive(Debug)]
struct Oracle {
    exclude: bool,
    folder: bool,
    anchored: bool,
    components: Vec<OracleComponent>,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Sep,
    Star,
    Any,
    Lit(char),
    /// A regex character class.
    Class(String),
}

fn escape(c: char) -> String {
    regex::escape(&c.to_string())
}

fn tokenize(body: &[char]) -> Result<Vec<(Token, char)>, OracleError> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        match c {
            '\\' | '/' => tokens.push((Token::Sep, c)),
            '*' => tokens.push((Token::Star, c)),
            '?' => tokens.push((Token::Any, c)),
            '[' => {
                let mut j = i + 1;
                let negated = body.get(j) == Some(&'!');
                if negated {
                    j += 1;
                }
                let content_start = j;
                if body.get(j) == Some(&']') {
                    j += 1;
                }
                let close = (j..body.len())
                    .find(|&k| body[k] == ']')
                    .ok_or(OracleError::Unclosed)?;
                let content = &body[content_start..close];
                let mut class = String::from(if negated { "[^" } else { "[" });
                let mut k = 0;
                while k < content.len() {
                    if k + 2 < content.len() && content[k + 1] == '-' {
                        if content[k] > content[k + 2] {
                            return Err(OracleError::Range);
                        }
                        class.push_str(&format!(
                            "{}-{}",
                            escape(content[k]),
                            escape(content[k + 2])
                        ));
                        k += 3;
                    } else {
                        class.push_str(&escape(content[k]));
                        k += 1;
                    }
                }
                class.push(']');
                tokens.push((Token::Class(class), '['));
                i = close;
            }
            _ => tokens.push((Token::Lit(c), c)),
        }
        i += 1;
    }
    Ok(tokens)
}

fn regex_of(tokens: &[Token], insensitive: bool) -> Regex {
    let mut text = String::from(if insensitive { "(?i)^(?s:" } else { "^(?s:" });
    for token in tokens {
        match token {
            Token::Star => text.push_str(".*"),
            Token::Any => text.push('.'),
            Token::Lit(c) => text.push_str(&escape(*c)),
            Token::Class(class) => text.push_str(class),
            Token::Sep => unreachable!("separators split components"),
        }
    }
    text.push_str(")$");
    Regex::new(&text).unwrap()
}

fn oracle_no_extension(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    !(1..chars.len().saturating_sub(1)).any(|k| chars[k] == '.')
}

impl OraclePattern {
    fn new(tokens: &[Token], insensitive: bool) -> OraclePattern {
        let n = tokens.len();
        let rule = if n >= 1 && tokens[n - 1] == Token::Lit('.') {
            Rule::TrailingDot
        } else if n >= 2 && tokens[n - 2] == Token::Lit('.') && tokens[n - 1] == Token::Star {
            Rule::DotStar
        } else {
            Rule::Plain
        };
        let shortened = match rule {
            Rule::Plain => None,
            Rule::TrailingDot => Some(regex_of(&tokens[..n - 1], insensitive)),
            Rule::DotStar => Some(regex_of(&tokens[..n - 2], insensitive)),
        };
        OraclePattern {
            full: regex_of(tokens, insensitive),
            shortened,
            rule,
        }
    }

    fn matches(&self, name: &str) -> bool {
        match self.rule {
            Rule::Plain => self.full.is_match(name),
            Rule::TrailingDot => {
                self.shortened.as_ref().unwrap().is_match(name) && oracle_no_extension(name)
            }
            Rule::DotStar => {
                self.full.is_match(name)
                    || (self.shortened.as_ref().unwrap().is_match(name)
                        && oracle_no_extension(name))
            }
        }
    }
}

fn oracle_parse(item: &str, insensitive: bool) -> Result<Oracle, OracleError> {
    let trimmed = item.trim_matches(|c| c == ' ' || c == '\t');
    let chars: Vec<char> = trimmed.chars().collect();
    let (exclude, body) = match chars.split_first() {
        Some(('-', rest)) => (true, rest),
        _ => (false, &chars[..]),
    };
    if body.is_empty() {
        return Err(OracleError::Empty);
    }
    let tokens = tokenize(body)?;
    let mut groups: Vec<Vec<(Token, char)>> = vec![Vec::new()];
    for token in tokens {
        if token.0 == Token::Sep {
            groups.push(Vec::new());
        } else {
            groups.last_mut().unwrap().push(token);
        }
    }
    let leading = groups.len() > 1 && groups[0].is_empty();
    let trailing = groups.len() > 1 && groups.last().unwrap().is_empty();
    if trailing {
        groups.pop();
    }
    if leading && !groups.is_empty() {
        groups.remove(0);
    }
    if groups.is_empty() || groups.iter().any(Vec::is_empty) {
        return Err(OracleError::Empty);
    }
    let raw = |group: &Vec<(Token, char)>| -> String {
        if group.iter().all(|(t, _)| matches!(t, Token::Lit(_))) {
            group.iter().map(|(_, c)| *c).collect()
        } else {
            String::new()
        }
    };
    let mut dot = false;
    for (k, group) in groups.iter().enumerate() {
        let text = raw(group);
        if text == "." && k == 0 {
            dot = true;
        } else if text == "." || text == ".." {
            return Err(OracleError::Dot);
        }
    }
    if dot {
        groups.remove(0);
        if groups.is_empty() {
            return Err(OracleError::Empty);
        }
    }
    let components: Vec<OracleComponent> = groups
        .iter()
        .map(|group| {
            if raw(group) == "..." {
                OracleComponent::AnyDepth
            } else {
                let tokens: Vec<Token> = group.iter().map(|(t, _)| t.clone()).collect();
                OracleComponent::Pattern(OraclePattern::new(&tokens, insensitive))
            }
        })
        .collect();
    let anchored = !matches!(components[0], OracleComponent::AnyDepth)
        && (leading || dot || components.len() > 1);
    Ok(Oracle {
        exclude,
        folder: trailing,
        anchored,
        components,
    })
}

impl Oracle {
    fn simple(&self) -> Option<&OraclePattern> {
        match self.components.as_slice() {
            [OracleComponent::Pattern(p)] if !self.anchored => Some(p),
            _ => None,
        }
    }

    fn path_match(components: &[OracleComponent], path: &[&str]) -> bool {
        match components.split_first() {
            None => path.is_empty(),
            Some((OracleComponent::AnyDepth, rest)) => {
                (0..=path.len()).any(|k| Self::path_match(rest, &path[k..]))
            }
            Some((OracleComponent::Pattern(p), rest)) => {
                !path.is_empty() && p.matches(path[0]) && Self::path_match(rest, &path[1..])
            }
        }
    }

    fn matches(&self, parent: &[String], name: &str) -> bool {
        match self.simple() {
            Some(p) => p.matches(name),
            None => {
                let mut path: Vec<&str> = parent.iter().map(String::as_str).collect();
                path.push(name);
                Self::path_match(&self.components, &path)
            }
        }
    }
}

fn error_kind(e: &MaskError) -> OracleError {
    match e {
        MaskError::UnclosedBracket { .. } => OracleError::Unclosed,
        MaskError::InvalidRange { .. } => OracleError::Range,
        MaskError::EmptyComponent { .. } => OracleError::Empty,
        MaskError::DotComponent { .. } => OracleError::Dot,
        other => panic!("unexpected error {other:?}"),
    }
}

fn options(insensitive: bool) -> MaskOptions {
    MaskOptions {
        case: if insensitive {
            CaseSensitivity::Insensitive
        } else {
            CaseSensitivity::Sensitive
        },
    }
}

fn rel(components: &[String]) -> RelPath {
    components
        .iter()
        .fold(RelPath::root(), |p, c| p.join(c).unwrap())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".."
}

// ---------------------------------------------------------------------------------------------------------
// Unicode strategies
// ---------------------------------------------------------------------------------------------------------

/// Names mixing case pairs, NFC/NFD forms and irregular case mappings, including sequences whose case fold does
/// not commute with NFC (`I\u{307}` composes to `İ`, which folds to itself; `W\u{30A}` folds to `w\u{30A}`,
/// which composes to `ẘ`) and chars that are neither lower case nor the upper case of their lower case (`ǅ`, `ẞ`,
/// `ϴ`).
fn unicode_name() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec![
            "a", "B", "c", ".", "-", "é", "É", "e\u{301}", "E\u{301}", "ß", "ẞ", "Σ", "σ", "ς",
            "\u{212A}", "k", "K", "ſ", "s", "S", "İ", "ı", "i", "I", "ǅ", "ǆ", "Ǆ", "Å",
            "\u{212B}", "a\u{30A}", "Ω", "\u{2126}", "ω", "I\u{307}", "i\u{307}", "W\u{30A}",
            "w\u{30A}", "ẘ", "J\u{30C}", "ǰ", "ϴ", "θ", "Θ",
        ]),
        1..7,
    )
    .prop_map(|parts| parts.concat())
}

/// True when folding `name` keeps its meaning under the §2.2.2 rules, which compare the **NFC** chars of a name
/// with `simple_lower` and test ranges through `{c, simple_lower(c), simple_upper(c)}`: folding commutes with
/// NFC, and every NFC char is its own lower case or the upper case of its lower case. Irregular names are
/// rare (`I\u{307}`, `W\u{30A}`, `ǅ`, `ẞ`, `ϴ`); for them the invariance holds for their NFC form instead.
fn fold_is_regular(name: &str) -> bool {
    let composed = nfc(name);
    nfc(&fold(name)) == fold(&composed)
        && composed.chars().all(|c| {
            let lower = simple_lower(c);
            c == lower || c == simple_upper(lower)
        })
}

/// Components usable in a `RelPath`.
fn unicode_component() -> impl Strategy<Value = String> {
    unicode_name().prop_map(common::valid_component)
}

/// Masks with Unicode literals and sets (members and ranges also in NFD and as singleton decompositions).
fn unicode_mask_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec![
            "a",
            "b",
            ".",
            "*",
            "?",
            "é",
            "É",
            "e\u{301}",
            "ß",
            "k",
            "s",
            "[é]",
            "[a-z]",
            "[!a-c]",
            "[À-ÿ]",
            "[A-Z]",
            "[σς]",
            "[k]",
            "[e\u{301}]",
            "[!E\u{301}]",
            "[\u{212A}]",
            "[a-e\u{301}]",
            "[ǅ]",
            "[Ǆ-ǆ]",
            "[θ-ϴ]",
            "i",
            "w\u{30A}",
        ]),
        1..6,
    )
    .prop_map(|parts| parts.concat())
}

/// Simple masks shaped like format masks (stars, literal prefixes and suffixes).
fn shaped_mask_text() -> impl Strategy<Value = String> {
    let lit = proptest::collection::vec(prop::sample::select(vec!['a', 'b', '.', 'A']), 0..4)
        .prop_map(|v| v.into_iter().collect::<String>());
    prop_oneof![
        (lit.clone()).prop_map(|s| format!("*{s}")),
        (lit.clone()).prop_map(|s| format!("{s}*")),
        (lit.clone(), lit.clone()).prop_map(|(a, b)| format!("{a}*{b}")),
        lit.clone()
            .prop_map(|s| if s.is_empty() { "a".to_owned() } else { s }),
        Just("*.*".to_owned()),
        Just("*".to_owned()),
        (lit.clone()).prop_map(|s| format!("{s}.*")),
        (lit).prop_map(|s| format!("{s}.")),
        common::simple_mask_text(),
    ]
}

// ---------------------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_mask.txt"))]

    /// P-MASK-1: Hikasa's matcher equals the regex oracle on random masks over `{a, b, ., *, ?, [, ], !, -}`
    /// and random names, in both case modes; `matches(parent, name) == matches_path(parent.join(name))`.
    #[test]
    fn p_mask_1_differential(
        mask in common::simple_mask_text(),
        names in proptest::collection::vec(common::ascii_name(), 1..6),
        parent in proptest::collection::vec(common::rel_component(), 0..3),
        insensitive in any::<bool>(),
    ) {
        let ours = Mask::parse(&mask, options(insensitive));
        let oracle = oracle_parse(&mask, insensitive);
        match (&ours, &oracle) {
            (Ok(m), Ok(o)) => {
                prop_assert_eq!(m.polarity() == Polarity::Exclude, o.exclude);
                prop_assert_eq!(m.target() == MaskTarget::Folders, o.folder);
                prop_assert_eq!(m.is_simple(), o.simple().is_some());
                let parent_path = rel(&parent);
                for name in &names {
                    let expected_name = o.simple().is_some_and(|p| p.matches(name));
                    prop_assert_eq!(m.matches_name(name), expected_name, "matches_name {:?} {:?}", mask, name);
                    if valid_name(name) {
                        let expected = o.matches(&parent, name);
                        prop_assert_eq!(m.matches(&parent_path, name), expected, "{:?} vs {:?}/{:?}", mask, parent, name);
                        let joined = parent_path.join(name).unwrap();
                        prop_assert_eq!(m.matches_path(&joined), expected);
                    }
                }
            }
            (Err(e), Err(oe)) => prop_assert_eq!(error_kind(e), *oe, "{:?}", mask),
            _ => prop_assert!(false, "parse disagreement for {:?}: {:?} vs {:?}", mask, ours.err(), oracle.err()),
        }
    }

    /// P-MASK-1 (paths): relative, anchored and `...` masks against random paths.
    #[test]
    fn p_mask_1_paths(
        mask in common::path_mask_text(),
        paths in proptest::collection::vec(proptest::collection::vec(common::rel_component(), 1..5), 1..6),
        insensitive in any::<bool>(),
    ) {
        let ours = Mask::parse(&mask, options(insensitive));
        let oracle = oracle_parse(&mask, insensitive);
        match (&ours, &oracle) {
            (Ok(m), Ok(o)) => {
                prop_assert_eq!(m.target() == MaskTarget::Folders, o.folder);
                prop_assert_eq!(m.is_simple(), o.simple().is_some());
                for path in &paths {
                    let (name, parent) = path.split_last().unwrap();
                    let parent_path = rel(parent);
                    let expected = o.matches(parent, name);
                    prop_assert_eq!(m.matches(&parent_path, name), expected, "{:?} vs {:?}", mask, path);
                    prop_assert_eq!(m.matches_path(&rel(path)), expected);
                }
            }
            (Err(e), Err(oe)) => prop_assert_eq!(error_kind(e), *oe, "{:?}", mask),
            _ => prop_assert!(false, "parse disagreement for {:?}: {:?} vs {:?}", mask, ours.err(), oracle.err()),
        }
    }

    /// `may_match_below` never misses a deeper match (no false negatives, D-09 traversal).
    #[test]
    fn may_match_below_is_sound(
        mask in common::path_mask_text(),
        folder in proptest::collection::vec(common::rel_component(), 1..4),
        below in proptest::collection::vec(proptest::collection::vec(common::rel_component(), 1..3), 1..6),
        insensitive in any::<bool>(),
    ) {
        let (Ok(m), Ok(o)) = (Mask::parse(&mask, options(insensitive)), oracle_parse(&mask, insensitive)) else {
            return Ok(());
        };
        let (name, parent) = folder.split_last().unwrap();
        let claims = m.may_match_below(&rel(parent), name);
        for tail in &below {
            let mut deeper = folder.clone();
            deeper.extend(tail.iter().cloned());
            let (leaf, leaf_parent) = deeper.split_last().unwrap();
            if o.matches(leaf_parent, leaf) {
                prop_assert!(claims, "{:?} matches {:?} below {:?}", mask, deeper, folder);
            }
        }
    }

    /// P-MASK-2: `MaskList::parse ∘ to_list_string` is the identity.
    #[test]
    fn p_mask_2_list_round_trip(
        items in proptest::collection::vec((common::path_mask_text(), prop::sample::select(vec!["", " ", "\t", "  "])), 0..6),
        insensitive in any::<bool>(),
    ) {
        let text: Vec<String> = items.iter().map(|(item, pad)| format!("{pad}{item}{pad}")).collect();
        let Ok(list) = MaskList::parse(&text.join(";"), options(insensitive)) else { return Ok(()); };
        let printed = list.to_list_string();
        let again = MaskList::parse(&printed, options(insensitive)).unwrap();
        prop_assert_eq!(again.to_list_string(), printed);
        prop_assert_eq!(again.masks().len(), list.masks().len());
        for (a, b) in list.masks().iter().zip(again.masks()) {
            prop_assert_eq!(a.source(), b.source());
            prop_assert_eq!(a.target(), b.target());
            prop_assert_eq!(a.polarity(), b.polarity());
            prop_assert_eq!(a.is_simple(), b.is_simple());
        }
    }

    /// `MaskList::matches_name` (which skips masks by their literal first and last bytes) is exactly "some
    /// include item matches the name" (D-10), in both case modes.
    #[test]
    fn mask_list_matches_name_equals_any_include_item(
        items in proptest::collection::vec(prop_oneof![common::simple_mask_text(), common::path_mask_text(), unicode_mask_text()], 0..8),
        names in proptest::collection::vec(prop_oneof![common::ascii_name(), unicode_name()], 1..8),
        insensitive in any::<bool>(),
    ) {
        let Ok(list) = MaskList::parse(&items.join(";"), options(insensitive)) else { return Ok(()); };
        for name in &names {
            let expected = list
                .masks()
                .iter()
                .any(|m| m.polarity() == Polarity::Include && m.matches_name(name));
            prop_assert_eq!(list.matches_name(name), expected, "{:?} vs {:?}", items, name);
        }
    }

    /// P-MASK-3: case-insensitive matches are invariant under `fold(name)`, for every name whose fold keeps its
    /// meaning under §2.2.2 ([`fold_is_regular`]); an irregular name matches exactly like its NFC form.
    #[test]
    fn p_mask_3_fold_invariance(
        mask in prop_oneof![common::simple_mask_text(), unicode_mask_text(), common::path_mask_text()],
        names in proptest::collection::vec(unicode_name(), 1..6),
        parent in proptest::collection::vec(unicode_component(), 0..3),
    ) {
        let Ok(m) = Mask::parse(&mask, options(true)) else { return Ok(()); };
        let parent_path = rel(&parent);
        let folded_parent = rel(&parent.iter().map(|c| common::valid_component(fold(c).into_owned())).collect::<Vec<_>>());
        let regular_parent = parent.iter().all(|c| fold_is_regular(c));
        for name in &names {
            let expected = m.matches(&parent_path, name);
            if fold_is_regular(name) {
                let folded = fold(name);
                prop_assert_eq!(expected, m.matches(&parent_path, &folded), "{:?} vs {:?}", mask, name);
                prop_assert_eq!(m.matches_name(name), m.matches_name(&folded));
            } else {
                let composed = nfc(name);
                prop_assert_eq!(expected, m.matches(&parent_path, &composed), "{:?} vs {:?} (irregular)", mask, name);
                prop_assert_eq!(m.matches_name(name), m.matches_name(&composed));
            }
            if regular_parent {
                prop_assert_eq!(expected, m.matches(&folded_parent, name), "parent {:?}", parent);
            }
        }
    }

    /// P-MASK-4: NFC invariance, `match(m, n) == match(m, nfd(n))` (names and parent components).
    #[test]
    fn p_mask_4_nfc_invariance(
        mask in prop_oneof![unicode_mask_text(), common::path_mask_text()],
        names in proptest::collection::vec(unicode_name(), 1..6),
        parent in proptest::collection::vec(unicode_component(), 0..3),
        insensitive in any::<bool>(),
    ) {
        let Ok(m) = Mask::parse(&mask, options(insensitive)) else { return Ok(()); };
        let nfd = |s: &str| s.nfd().collect::<String>();
        let parent_path = rel(&parent);
        let nfd_parent = rel(&parent.iter().map(|c| nfd(c)).collect::<Vec<_>>());
        for name in &names {
            let decomposed = nfd(name);
            prop_assert_eq!(m.matches(&parent_path, name), m.matches(&parent_path, &decomposed), "{:?} vs {:?}", mask, name);
            prop_assert_eq!(m.matches(&parent_path, name), m.matches(&nfd_parent, name));
            prop_assert_eq!(m.matches_name(name), m.matches_name(&decomposed));
        }
        // The mask itself may be typed in NFD as well: literals, set members and range bounds (D-07).
        let m_nfd = Mask::parse(&nfd(&mask), options(insensitive)).unwrap();
        prop_assert_eq!(m_nfd.source(), nfd(mask.trim_matches([' ', '\t'])));
        for name in &names {
            prop_assert_eq!(m.matches(&parent_path, name), m_nfd.matches(&parent_path, name), "{:?} in NFD vs {:?}", mask, name);
        }
    }

    /// Containment soundness (§2.12.3): `covered(i, j) ∧ match(i, n) ⇒ match(j, n)`.
    #[test]
    fn containment_is_sound(
        i in shaped_mask_text(),
        j in shaped_mask_text(),
        names in proptest::collection::vec(prop_oneof![common::ascii_name(), unicode_name()], 1..10),
        insensitive_i in any::<bool>(),
        insensitive_j in any::<bool>(),
    ) {
        let (Ok(mi), Ok(mj)) = (Mask::parse(&i, options(insensitive_i)), Mask::parse(&j, options(insensitive_j))) else {
            return Ok(());
        };
        if mi.is_covered_by(&mj) {
            for name in &names {
                if mi.matches_name(name) {
                    prop_assert!(mj.matches_name(name), "{:?} covered by {:?} but {:?} differs", i, j, name);
                }
            }
        }
    }
}

#[test]
fn every_mask_covers_itself() {
    for text in [
        "*.c", "a?b", "[a-c]x", "abc.", "x.*", "*", "src\\*.c", "...\\x",
    ] {
        for insensitive in [false, true] {
            let m = Mask::parse(text, options(insensitive)).unwrap();
            assert!(m.is_covered_by(&m), "{text}");
        }
    }
}
