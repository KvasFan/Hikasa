//! The file mask engine (FC-050, FC-051; algorithm §2.2; D-07, D-08).
//!
//! ```text
//! list       := item (";" item)*                      empty items (after trimming) are ignored
//! item       := WS* ["-"] body WS*                   WS = ASCII space or tab; "-" = exclude
//! body       := [sep] components [sep]               a trailing sep marks a FOLDER mask; a leading sep anchors
//! components := comp (sep comp)*                     sep = "\" or "/"
//! comp       := "..." (zero or more folders) | "." (base folder, first component only) | pattern
//! pattern    := ("?" | "*" | set | literal-char)+
//! set        := "[" ["!"] setbody "]"                 "]" first is literal; "a-z" is a range; "-" first or last is
//!                                                     literal; "\" and "/" inside a set are literal
//! ```
//!
//! * Names and masks are compared in NFC: a name is converted before matching and a mask item before it is
//!   parsed (literals, set members and range bounds); case-insensitive matching uses the simple 1:1 folding
//!   of [`crate::case`] on the NFC chars (D-07, §2.2.2).
//! * A trailing `.` means "no extension"; a mask ending in `.*` also matches names without an extension.
//! * A mask without separators ("simple") matches the item name at any depth. A relative mask (`p\f`, `.\f`,
//!   `\f`) is anchored at the base folder; `...` matches zero or more folders anywhere (D-08).
//!
//! ```
//! use hikasa_core::mask::{CaseSensitivity, Mask, MaskOptions};
//! use hikasa_core::path::RelPath;
//!
//! let opts = MaskOptions { case: CaseSensitivity::Insensitive };
//! let mask = Mask::parse(r"src\...\*.rs", opts).unwrap();
//! assert!(mask.matches_path(&RelPath::parse("src/a/b/x.RS").unwrap()));
//! assert!(!mask.matches_path(&RelPath::parse("lib/src/x.rs").unwrap()));
//! ```

mod error;
mod glob;

use serde::{Deserialize, Serialize};

pub use error::MaskError;

use crate::case::nfc;
use crate::path::RelPath;
pub(crate) use glob::Name;
use glob::{Glob, NameBounds, check_set, set_end};

/// Case sensitivity of name matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseSensitivity {
    /// Chars must be equal.
    Sensitive,
    /// Chars are equal when their simple lower cases are equal (D-07).
    Insensitive,
}

impl CaseSensitivity {
    /// Platform default for local names: Insensitive on Windows and macOS, Sensitive elsewhere (FC-010).
    pub const HOST_DEFAULT: CaseSensitivity = if cfg!(any(windows, target_os = "macos")) {
        CaseSensitivity::Insensitive
    } else {
        CaseSensitivity::Sensitive
    };
}

/// Options shared by every mask in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaskOptions {
    /// Case sensitivity (D-07: simple 1:1 folding).
    pub case: CaseSensitivity,
}

/// What a mask item applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskTarget {
    /// No trailing separator.
    Files,
    /// Trailing `\` or `/`.
    Folders,
}

/// Include (`*.c`) or exclude (`-*.bak`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// No leading `-`.
    Include,
    /// Leading `-`.
    Exclude,
}

/// One path component of a mask.
#[derive(Debug, Clone)]
pub(crate) enum ComponentPattern {
    /// `...`: zero or more components.
    AnyDepth,
    /// One component.
    Pattern(Glob),
}

impl ComponentPattern {
    fn same_structure(&self, other: &ComponentPattern) -> bool {
        match (self, other) {
            (ComponentPattern::AnyDepth, ComponentPattern::AnyDepth) => true,
            (ComponentPattern::Pattern(a), ComponentPattern::Pattern(b)) => a.same_structure(b),
            (ComponentPattern::AnyDepth, ComponentPattern::Pattern(_))
            | (ComponentPattern::Pattern(_), ComponentPattern::AnyDepth) => false,
        }
    }
}

/// One compiled mask item.
#[derive(Debug, Clone)]
pub struct Mask {
    /// The item exactly as typed (after trimming), for round-tripping.
    source: String,
    target: MaskTarget,
    polarity: Polarity,
    /// True for `p\f`, `.\f`, `\f`; false for `f` and `...\f`.
    anchored: bool,
    /// [`ComponentPattern::AnyDepth`] for `...`.
    components: Vec<ComponentPattern>,
    opts: MaskOptions,
}

/// Trims ASCII spaces and tabs (the only item whitespace, §2.2.1).
pub(crate) fn trim_item(item: &str) -> &str {
    item.trim_matches([' ', '\t'])
}

/// Position in the sequence "parent components, then the name, then `extra` wildcard components".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cursor {
    /// A parent component starting at this byte offset of the parent text.
    Parent(usize),
    /// The item's own name.
    Name,
    /// The k-th wildcard component (matches any single-component pattern).
    Extra(u32),
    /// Past the end.
    End,
}

/// The components of `parent/name` (+ wildcards), walked in place without building a joined path.
struct PathSeq<'a> {
    parent: &'a str,
    name: Name<'a>,
    extra: u32,
}

impl PathSeq<'_> {
    fn first(&self) -> Cursor {
        if self.parent.is_empty() {
            Cursor::Name
        } else {
            Cursor::Parent(0)
        }
    }

    fn next(&self, cursor: Cursor) -> Cursor {
        match cursor {
            Cursor::Parent(offset) => {
                match self.parent.get(offset..).and_then(|rest| rest.find('/')) {
                    Some(slash) => Cursor::Parent(offset + slash + 1),
                    None => Cursor::Name,
                }
            }
            Cursor::Name => {
                if self.extra > 0 {
                    Cursor::Extra(0)
                } else {
                    Cursor::End
                }
            }
            Cursor::Extra(k) => {
                if k + 1 < self.extra {
                    Cursor::Extra(k + 1)
                } else {
                    Cursor::End
                }
            }
            Cursor::End => Cursor::End,
        }
    }

    fn matches(&self, glob: &Glob, cursor: Cursor) -> bool {
        match cursor {
            Cursor::Parent(offset) => {
                let rest = self.parent.get(offset..).unwrap_or("");
                let end = rest.find('/').unwrap_or(rest.len());
                glob.matches(rest.get(..end).unwrap_or(""))
            }
            Cursor::Name => glob.matches_nfc(self.name),
            Cursor::Extra(_) => true,
            Cursor::End => false,
        }
    }
}

impl Mask {
    /// Parses one item (without `;`). Leading/trailing ASCII space and tab are trimmed.
    ///
    /// Masks are compared in NFC (D-07): the trimmed item is converted to NFC before it is parsed, so literals,
    /// set members and range bounds typed in NFD (a mask pasted from a macOS file name) or as singleton
    /// decompositions (KELVIN SIGN) mean their NFC characters, exactly like the names they are matched
    /// against. NFC never adds, removes or reorders the syntax characters, so the structure is the item's as
    /// typed; [`Mask::source`] keeps the item as typed, and error positions count the chars of the NFC text.
    pub fn parse(item: &str, opts: MaskOptions) -> Result<Mask, MaskError> {
        let source = trim_item(item);
        let normalized = nfc(source);
        let nfc_item: &str = &normalized;
        let chars: Vec<char> = nfc_item.chars().collect();
        let empty = || MaskError::EmptyComponent {
            mask: nfc_item.to_owned(),
        };
        let (polarity, body_start) = if chars.first() == Some(&'-') {
            (Polarity::Exclude, 1)
        } else {
            (Polarity::Include, 0)
        };
        if body_start >= chars.len() {
            return Err(empty());
        }
        // Split the body at separators outside sets.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut start = body_start;
        let mut i = body_start;
        while i < chars.len() {
            match chars.get(i) {
                Some('\\' | '/') => {
                    ranges.push((start, i));
                    i += 1;
                    start = i;
                }
                Some('[') => {
                    // Set errors are reported in text order, before any component error.
                    let close = set_end(&chars, i, chars.len()).ok_or_else(|| {
                        MaskError::UnclosedBracket {
                            mask: nfc_item.to_owned(),
                            at: i,
                        }
                    })?;
                    check_set(&chars, i, close, nfc_item)?;
                    i = close + 1;
                }
                _ => i += 1,
            }
        }
        ranges.push((start, chars.len()));
        let leading_sep = ranges.len() > 1 && ranges.first().is_some_and(|&(a, b)| a == b);
        let trailing_sep = ranges.len() > 1 && ranges.last().is_some_and(|&(a, b)| a == b);
        if trailing_sep {
            ranges.pop();
        }
        if leading_sep && !ranges.is_empty() {
            ranges.remove(0);
        }
        if ranges.is_empty() || ranges.iter().any(|&(a, b)| a == b) {
            return Err(empty());
        }
        let text = |&(a, b): &(usize, usize)| chars.get(a..b).unwrap_or(&[]);
        let mut had_dot = false;
        for (k, range) in ranges.iter().enumerate() {
            match text(range) {
                ['.'] if k == 0 => had_dot = true,
                ['.'] | ['.', '.'] => {
                    return Err(MaskError::DotComponent {
                        mask: nfc_item.to_owned(),
                    });
                }
                _ => {}
            }
        }
        if had_dot {
            ranges.remove(0);
            if ranges.is_empty() {
                return Err(empty());
            }
        }
        let insensitive = opts.case == CaseSensitivity::Insensitive;
        let mut components = Vec::with_capacity(ranges.len());
        for range in &ranges {
            if text(range) == ['.', '.', '.'] {
                components.push(ComponentPattern::AnyDepth);
            } else {
                components.push(ComponentPattern::Pattern(Glob::compile(
                    &chars,
                    range.0,
                    range.1,
                    nfc_item,
                    insensitive,
                )?));
            }
        }
        let starts_any_depth = matches!(components.first(), Some(ComponentPattern::AnyDepth));
        let anchored = !starts_any_depth && (leading_sep || had_dot || components.len() > 1);
        let target = if trailing_sep {
            MaskTarget::Folders
        } else {
            MaskTarget::Files
        };
        Ok(Mask {
            source: source.to_owned(),
            target,
            polarity,
            anchored,
            components,
            opts,
        })
    }

    /// The item as typed (after trimming).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Files or folders.
    pub fn target(&self) -> MaskTarget {
        self.target
    }

    /// Include or exclude.
    pub fn polarity(&self) -> Polarity {
        self.polarity
    }

    /// True when the mask has no separators (matches a name at any depth).
    pub fn is_simple(&self) -> bool {
        !self.anchored && matches!(self.components.as_slice(), [ComponentPattern::Pattern(_)])
    }

    /// True for anchored masks (`p\f`, `.\f`, `\f`): they match relative paths from the base folder.
    pub(crate) fn is_anchored(&self) -> bool {
        self.anchored
    }

    /// True for a simple mask that matches every name (`*`, `*.*`, …).
    pub(crate) fn is_universal(&self) -> bool {
        self.simple_glob().is_some_and(Glob::is_universal)
    }

    fn simple_glob(&self) -> Option<&Glob> {
        match self.components.as_slice() {
            [ComponentPattern::Pattern(glob)] if !self.anchored => Some(glob),
            _ => None,
        }
    }

    /// Matches a bare name (only meaningful for simple masks; relative masks return false).
    pub fn matches_name(&self, name: &str) -> bool {
        self.simple_glob().is_some_and(|glob| glob.matches(name))
    }

    /// [`Mask::matches_name`] for a prepared NFC name.
    pub(crate) fn matches_name_nfc(&self, name: Name<'_>) -> bool {
        self.simple_glob()
            .is_some_and(|glob| glob.matches_nfc(name))
    }

    /// Full semantics (§2.2.3) against the item `parent/name`, where `parent` is the item's folder relative to
    /// the base (root = the base folder). Ignores `target`. Allocation-free for ASCII and NFC names: components
    /// are compared in place, folding is applied per char, and `case::nfc` borrows (bench `mask_match`, §6.4).
    pub fn matches(&self, parent: &RelPath, name: &str) -> bool {
        let name = nfc(name);
        self.matches_prepared(parent.as_str(), Name::new(&name))
    }

    /// The condition on the item's own name that [`Mask::matches_prepared`] checks first: the bounds of the
    /// pattern the name itself must match (the only pattern of a simple mask, the last pattern of a path mask);
    /// open when the mask ends with `...`.
    fn name_bounds(&self) -> NameBounds {
        match self.components.last() {
            Some(ComponentPattern::Pattern(glob)) => glob.bounds(),
            Some(ComponentPattern::AnyDepth) | None => NameBounds::OPEN,
        }
    }

    /// [`Mask::matches`] with the parent's text and a prepared NFC name.
    pub(crate) fn matches_prepared(&self, parent: &str, name: Name<'_>) -> bool {
        match self.simple_glob() {
            Some(glob) => glob.matches_nfc(name),
            None => {
                // When the mask ends with a pattern, that pattern is the one the item's own name must match.
                if let Some(ComponentPattern::Pattern(last)) = self.components.last()
                    && !last.may_match(name)
                {
                    return false;
                }
                self.path_match(parent, name, 0)
            }
        }
    }

    /// Convenience: `matches(path.parent(), path.name())`; the root never matches.
    pub fn matches_path(&self, path: &RelPath) -> bool {
        match path.name() {
            Some(name) => {
                let name = nfc(name);
                self.matches_prepared(path.parent_str(), Name::new(&name))
            }
            None => false,
        }
    }

    /// True when the folder `parent/name` is a proper prefix of some path this (anchored) mask could match
    /// (D-09 traversal). Allocation-free like `matches`. Unanchored masks can match below any folder, so they
    /// return true. The test is conservative: it may report a deeper match that no real name can produce,
    /// never the opposite.
    pub fn may_match_below(&self, parent: &RelPath, name: &str) -> bool {
        let name = nfc(name);
        self.may_match_below_prepared(parent.as_str(), Name::new(&name))
    }

    /// [`Mask::may_match_below`] with the parent's text and a prepared NFC name.
    pub(crate) fn may_match_below_prepared(&self, parent: &str, name: Name<'_>) -> bool {
        if !self.anchored {
            return true;
        }
        let patterns = self
            .components
            .iter()
            .filter(|c| matches!(c, ComponentPattern::Pattern(_)))
            .count();
        let most = u32::try_from(patterns.max(1)).unwrap_or(u32::MAX);
        (1..=most).any(|extra| self.path_match(parent, name, extra))
    }

    /// The greedy single-backtrack algorithm of §2.2.2 with components as atoms and `...` as the star.
    fn path_match(&self, parent: &str, name: Name<'_>, extra: u32) -> bool {
        let seq = PathSeq {
            parent,
            name,
            extra,
        };
        let patterns = self.components.as_slice();
        let (mut i, mut j) = (0usize, seq.first());
        let mut star: Option<usize> = None;
        let mut mark = j;
        while j != Cursor::End {
            match patterns.get(i) {
                Some(ComponentPattern::AnyDepth) => {
                    star = Some(i);
                    mark = j;
                    i += 1;
                    continue;
                }
                Some(ComponentPattern::Pattern(glob)) if seq.matches(glob, j) => {
                    i += 1;
                    j = seq.next(j);
                    continue;
                }
                Some(ComponentPattern::Pattern(_)) | None => {}
            }
            match star {
                Some(s) => {
                    i = s + 1;
                    mark = seq.next(mark);
                    j = mark;
                }
                None => return false,
            }
        }
        patterns
            .get(i..)
            .is_some_and(|rest| rest.iter().all(|c| matches!(c, ComponentPattern::AnyDepth)))
    }

    /// Sound containment test used for format shadowing: `true` only if every name matched by
    /// `self` is also matched by `other` (§2.12.3). May return false negatives, never false positives.
    ///
    /// Simple masks are analysed by the rules (a)–(e) of §2.12.3; other masks are covered only by a mask of
    /// identical structure. Targets and polarities are not compared: this is about the matched names.
    pub fn is_covered_by(&self, other: &Mask) -> bool {
        if let (Some(mine), Some(theirs)) = (self.simple_glob(), other.simple_glob()) {
            return mine.is_covered_by(theirs);
        }
        self.opts.case == other.opts.case
            && self.anchored == other.anchored
            && self.components.len() == other.components.len()
            && self
                .components
                .iter()
                .zip(&other.components)
                .all(|(a, b)| a.same_structure(b))
    }
}

/// A `;`-separated mask list (empty items are ignored).
#[derive(Debug, Clone)]
pub struct MaskList {
    masks: Vec<Mask>,
    opts: MaskOptions,
    /// `masks[k].name_bounds()`, kept contiguous: a name is first checked against every mask's two bytes, and
    /// only the masks it passes run (identical results, a fraction of the work for long lists).
    bounds: Box<[NameBounds]>,
}

impl MaskList {
    /// Parses a list.
    pub fn parse(list: &str, opts: MaskOptions) -> Result<MaskList, MaskError> {
        let mut masks = Vec::new();
        for item in list.split(';') {
            if trim_item(item).is_empty() {
                continue;
            }
            masks.push(Mask::parse(item, opts)?);
        }
        Ok(MaskList::from_masks(masks, opts))
    }

    /// A list of already parsed masks (parsed with `opts`).
    pub(crate) fn from_masks(masks: Vec<Mask>, opts: MaskOptions) -> MaskList {
        let bounds = masks.iter().map(Mask::name_bounds).collect();
        MaskList {
            masks,
            opts,
            bounds,
        }
    }

    /// True if `test` holds for some mask that may match a name with these [`Name::edges`]: masks whose
    /// [`NameBounds`] reject the edges cannot match and are skipped. Masks are tried in list order and the scan
    /// stops at the first hit.
    #[inline]
    fn any_candidate(&self, edges: Option<(u8, u8)>, test: impl FnMut(&Mask) -> bool) -> bool {
        self.masks
            .iter()
            .zip(self.bounds.iter())
            .filter(|(_, bounds)| bounds.admits(edges))
            .map(|(mask, _)| mask)
            .any(test)
    }

    /// True if any mask (of either polarity) matches the item `parent/name` ([`Mask::matches_prepared`]).
    pub(crate) fn any_matches_prepared(&self, parent: &str, name: Name<'_>) -> bool {
        let edges = name.edges(self.opts.case == CaseSensitivity::Insensitive);
        self.any_candidate(edges, |mask| mask.matches_prepared(parent, name))
    }

    /// No items.
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// Items in order.
    pub fn masks(&self) -> &[Mask] {
        &self.masks
    }

    /// Options used.
    pub fn options(&self) -> MaskOptions {
        self.opts
    }

    /// True if any *include* item matches `name` (used for file formats, which ignore `-` items: D-10).
    pub fn matches_name(&self, name: &str) -> bool {
        let name = nfc(name);
        let name = Name::new(&name);
        let edges = name.edges(self.opts.case == CaseSensitivity::Insensitive);
        self.any_candidate(edges, |mask| {
            mask.polarity == Polarity::Include && mask.matches_name_nfc(name)
        })
    }

    /// Canonical `;`-joined text (round-trips through `parse`, INV-MASK-2).
    pub fn to_list_string(&self) -> String {
        let mut out = String::new();
        for (k, mask) in self.masks.iter().enumerate() {
            if k > 0 {
                out.push(';');
            }
            out.push_str(&mask.source);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const I: MaskOptions = MaskOptions {
        case: CaseSensitivity::Insensitive,
    };

    #[test]
    fn structure_of_parsed_masks() {
        let m = Mask::parse("  -src\\*.c ", I).unwrap();
        assert_eq!(m.source(), "-src\\*.c");
        assert_eq!(m.polarity(), Polarity::Exclude);
        assert_eq!(m.target(), MaskTarget::Files);
        assert!(m.is_anchored());
        assert!(!m.is_simple());
        let simple = Mask::parse("obj/", I).unwrap();
        assert!(simple.is_simple());
        assert_eq!(simple.target(), MaskTarget::Folders);
        let deep = Mask::parse("...\\x", I).unwrap();
        assert!(!deep.is_anchored());
        assert!(!deep.is_simple());
        assert!(Mask::parse(".\\x", I).unwrap().is_anchored());
        assert!(Mask::parse("\\x", I).unwrap().is_anchored());
        assert!(
            Mask::parse("[/]x", I).unwrap().is_simple(),
            "separators inside sets are literal"
        );
    }

    #[test]
    fn errors() {
        let err = |s: &str| Mask::parse(s, I).unwrap_err();
        assert_eq!(
            err("a[b"),
            MaskError::UnclosedBracket {
                mask: "a[b".into(),
                at: 1
            }
        );
        assert_eq!(
            err("[z-a]"),
            MaskError::InvalidRange {
                mask: "[z-a]".into(),
                at: 1,
                lo: 'z',
                hi: 'a'
            }
        );
        assert_eq!(
            err("a\\\\b"),
            MaskError::EmptyComponent {
                mask: "a\\\\b".into()
            }
        );
        assert_eq!(err("-"), MaskError::EmptyComponent { mask: "-".into() });
        assert_eq!(err(""), MaskError::EmptyComponent { mask: "".into() });
        assert_eq!(err("\\"), MaskError::EmptyComponent { mask: "\\".into() });
        assert_eq!(err(".\\"), MaskError::EmptyComponent { mask: ".\\".into() });
        assert_eq!(
            err("x\\..\\y"),
            MaskError::DotComponent {
                mask: "x\\..\\y".into()
            }
        );
        assert_eq!(
            err("x\\.\\y"),
            MaskError::DotComponent {
                mask: "x\\.\\y".into()
            }
        );
        assert_eq!(err(".."), MaskError::DotComponent { mask: "..".into() });
    }

    /// `Mask::parse` converts the item to NFC before parsing it. That is sound only because NFC never adds,
    /// removes or moves a syntax character: they are starters (never reordered), no character decomposes into
    /// one, and none composes with a neighbour.
    #[test]
    fn nfc_never_touches_the_syntax_characters() {
        use unicode_normalization::char::{
            canonical_combining_class, compose, decompose_canonical,
        };
        const SYNTAX: [char; 11] = ['*', '?', '[', ']', '!', '-', '\\', '/', '.', ' ', '\t'];
        for s in SYNTAX {
            assert_eq!(canonical_combining_class(s), 0, "{s:?} is a starter");
        }
        for c in (0..=0x10_FFFF_u32).filter_map(char::from_u32) {
            for s in SYNTAX {
                assert_eq!(compose(c, s), None, "U+{:04X} + {s:?}", u32::from(c));
                assert_eq!(compose(s, c), None, "{s:?} + U+{:04X}", u32::from(c));
            }
            if !SYNTAX.contains(&c) {
                decompose_canonical(c, |d| {
                    assert!(
                        !SYNTAX.contains(&d),
                        "U+{:04X} decomposes into {d:?}",
                        u32::from(c)
                    );
                });
            }
        }
    }

    #[test]
    fn may_match_below_examples() {
        let root = RelPath::root();
        let m = Mask::parse("src\\core\\", I).unwrap();
        assert!(m.may_match_below(&root, "src"));
        assert!(!m.may_match_below(&root, "lib"));
        let src = RelPath::parse("src").unwrap();
        assert!(
            !m.may_match_below(&src, "core"),
            "src/core itself matches; nothing below it does"
        );
        let deep = Mask::parse("a\\...\\z\\", I).unwrap();
        assert!(deep.may_match_below(&root, "a"));
        assert!(deep.may_match_below(&RelPath::parse("a/b").unwrap(), "c"));
        assert!(!deep.may_match_below(&root, "b"));
        assert!(
            Mask::parse("x\\", I)
                .unwrap()
                .may_match_below(&root, "anything"),
            "unanchored"
        );
    }

    #[test]
    fn list_round_trip() {
        let list = MaskList::parse(" *.c ; ;-*.bak;src\\;", I).unwrap();
        assert_eq!(list.to_list_string(), "*.c;-*.bak;src\\");
        assert_eq!(list.masks().len(), 3);
        assert!(list.matches_name("X.C"));
        assert!(
            !list.matches_name("x.bak"),
            "exclude items never match in matches_name"
        );
        assert!(!list.is_empty());
        assert!(MaskList::parse(" ; ", I).unwrap().is_empty());
        assert_eq!(list.options(), I);
    }
}
