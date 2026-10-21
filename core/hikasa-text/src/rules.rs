//! Text rules (TC-010…TC-022, §1.5.3): [`TextRules`] (persisted TOML, `hikasa text compare --rules-file`),
//! [`ImportanceRules`], [`AlignmentRules`], [`AlgorithmChoice`], [`TextPattern`], the `--unimportant`
//! vocabulary [`UnimportantItem`] and the flag/IPC overrides [`TextRulesPatch`].
//!
//! The defaults are D-04: every whitespace class, every grammar element (unless the format ships it
//! unimportant) and "everything else" are important and case-sensitive; there are no unimportant-text
//! patterns; orphan lines are always important; line endings, encoding and BOM are not compared; the
//! algorithm is `Auto` (D-36), with skew 300, closeness on at 500 ‰ and never-align off. With these defaults a
//! comparison is exact apart from line endings, encoding and BOM, and the identity fast path (D-53) applies
//! whenever the format has no unimportant or case-insensitive element.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use hikasa_core::schema::check_schema_version;
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_diff::Algorithm;
use hikasa_formats::{CompiledFormat, ElementId, ElementName, Pattern, RegexLimits};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::TextError;

/// Schema version of rules files.
pub const TEXT_RULES_SCHEMA_VERSION: u32 = 1;

fn yes() -> bool {
    true
}

/// Session rules of a Text Compare (TC-010…TC-022). `Default` = D-04.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextRules {
    /// Schema version (SES-052).
    pub schema_version: u32,
    /// Importance tab.
    pub importance: ImportanceRules,
    /// Alignment tab.
    pub alignment: AlignmentRules,
}

impl Default for TextRules {
    /// D-04.
    fn default() -> TextRules {
        TextRules {
            schema_version: TEXT_RULES_SCHEMA_VERSION,
            importance: ImportanceRules::default(),
            alignment: AlignmentRules::default(),
        }
    }
}

/// Importance rules (TC-020…TC-022). Defaults (D-04) give an exact comparison apart from line endings,
/// encoding and BOM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportanceRules {
    /// Element name → important. Overrides the format's `ElementInfo::important` for that name; names not
    /// listed use the format's default. Default: empty.
    pub elements: BTreeMap<String, bool>,
    /// Leading whitespace important. Default true.
    pub leading_whitespace: bool,
    /// Embedded whitespace important. Default true.
    pub embedded_whitespace: bool,
    /// Trailing whitespace important. Default true.
    pub trailing_whitespace: bool,
    /// Text not covered by grammar items is important. Default true.
    pub everything_else: bool,
    /// Case changes in everything-else text are important. Default true.
    pub everything_else_case_sensitive: bool,
    /// "Unimportant text" patterns (TC-020), applied before everything else. Default: none.
    pub unimportant: Vec<TextPattern>,
    /// TC-021. Default true.
    pub orphans_always_important: bool,
    /// TC-022. Default false.
    pub compare_line_endings: bool,
}

impl Default for ImportanceRules {
    /// D-04.
    fn default() -> ImportanceRules {
        ImportanceRules {
            elements: BTreeMap::new(),
            leading_whitespace: true,
            embedded_whitespace: true,
            trailing_whitespace: true,
            everything_else: true,
            everything_else_case_sensitive: true,
            unimportant: Vec::new(),
            orphans_always_important: true,
            compare_line_endings: false,
        }
    }
}

impl ImportanceRules {
    /// Marks every item unimportant: whitespace classes and everything-else set their flag to false, `Case`
    /// sets `everything_else_case_sensitive = false`, `Element(n)` sets `elements[n] = false`.
    pub fn apply_unimportant(&mut self, items: &[UnimportantItem]) {
        for item in items {
            match item {
                UnimportantItem::LeadingWhitespace => self.leading_whitespace = false,
                UnimportantItem::EmbeddedWhitespace => self.embedded_whitespace = false,
                UnimportantItem::TrailingWhitespace => self.trailing_whitespace = false,
                UnimportantItem::Whitespace => {
                    self.leading_whitespace = false;
                    self.embedded_whitespace = false;
                    self.trailing_whitespace = false;
                }
                UnimportantItem::Case => self.everything_else_case_sensitive = false,
                UnimportantItem::EverythingElse => self.everything_else = false,
                UnimportantItem::Element(name) => {
                    self.elements.insert(name.clone(), false);
                }
            }
        }
    }
}

/// A plain or regex pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextPattern {
    /// Pattern text.
    pub pattern: String,
    /// Regex (engine chosen per §2.11).
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive. Default true.
    #[serde(default = "yes")]
    pub match_case: bool,
}

impl TextPattern {
    /// A plain, case-sensitive pattern (`--unimportant-text`).
    pub fn plain(text: impl Into<String>) -> TextPattern {
        TextPattern {
            pattern: text.into(),
            regex: false,
            match_case: true,
        }
    }

    /// A case-sensitive regular expression (`--unimportant-regex`).
    pub fn regex(source: impl Into<String>) -> TextPattern {
        TextPattern {
            pattern: source.into(),
            regex: true,
            match_case: true,
        }
    }

    /// Compiles the pattern (plain text is matched literally; §2.11 engine selection for regexes).
    pub(crate) fn compile(&self, limits: &RegexLimits) -> Result<Pattern, String> {
        if self.pattern.is_empty() {
            return Err("the pattern is empty".to_owned());
        }
        let source: Cow<'_, str> = if self.regex {
            Cow::Borrowed(&self.pattern)
        } else {
            Cow::Owned(regex::escape(&self.pattern))
        };
        Pattern::compile(&source, !self.match_case, limits).map_err(|e| e.to_string())
    }
}

/// One word of the `--unimportant` vocabulary, shared by the CLI and (M2) scripts (T11).
/// Serialised as its word (`"leading-ws"`, …, or `"element:<Name>"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum UnimportantItem {
    /// `leading-ws`.
    LeadingWhitespace,
    /// `embedded-ws`.
    EmbeddedWhitespace,
    /// `trailing-ws`.
    TrailingWhitespace,
    /// `whitespace` = the three classes above.
    Whitespace,
    /// `case` = everything-else text is compared case-insensitively.
    Case,
    /// `everything-else`.
    EverythingElse,
    /// Any other word, or `element:<Name>` (needed only when the name equals a vocabulary word): a grammar
    /// element name (exact, case-sensitive).
    Element(String),
}

/// The fixed words of the vocabulary, in declaration order.
const WORDS: [(&str, UnimportantItem); 6] = [
    ("leading-ws", UnimportantItem::LeadingWhitespace),
    ("embedded-ws", UnimportantItem::EmbeddedWhitespace),
    ("trailing-ws", UnimportantItem::TrailingWhitespace),
    ("whitespace", UnimportantItem::Whitespace),
    ("case", UnimportantItem::Case),
    ("everything-else", UnimportantItem::EverythingElse),
];

/// The explicit element prefix.
const ELEMENT_PREFIX: &str = "element:";

/// The reserved name of element 0 (`hikasa_formats::EVERYTHING_ELSE`): as a bare `--unimportant` word (any
/// ASCII case) it means [`UnimportantItem::EverythingElse`]; as an element name it is rejected.
const EVERYTHING_ELSE_NAME: &str = "Everything Else";

impl UnimportantItem {
    /// Parses a comma-separated list. Items are trimmed; empty items are ignored; the fixed words match
    /// ASCII-case-insensitively. An item that is not a valid `ElementName` is `TextError::Rules`.
    ///
    /// The reserved name `Everything Else` (any ASCII case) is the word `everything-else`; `element:Everything
    /// Else` is `TextError::Rules`, because no grammar element can have that name (§1.4.8).
    pub fn parse_list(list: &str) -> Result<Vec<UnimportantItem>, TextError> {
        let mut out = Vec::new();
        for raw in list.split(',') {
            let item = raw.trim();
            if item.is_empty() {
                continue;
            }
            out.push(UnimportantItem::parse_one(item)?);
        }
        Ok(out)
    }

    /// One trimmed, non-empty item.
    fn parse_one(item: &str) -> Result<UnimportantItem, TextError> {
        if let Some((_, word)) = WORDS
            .iter()
            .find(|(word, _)| word.eq_ignore_ascii_case(item))
        {
            return Ok(word.clone());
        }
        if item.eq_ignore_ascii_case(EVERYTHING_ELSE_NAME) {
            return Ok(UnimportantItem::EverythingElse);
        }
        let name = match item.get(..ELEMENT_PREFIX.len()) {
            Some(prefix) if prefix.eq_ignore_ascii_case(ELEMENT_PREFIX) => {
                &item[ELEMENT_PREFIX.len()..]
            }
            _ => item,
        };
        if name.trim() == EVERYTHING_ELSE_NAME {
            return Err(TextError::Rules(format!(
                "invalid unimportant item {item:?}: \"{EVERYTHING_ELSE_NAME}\" is not an element name; use \
                 `everything-else`"
            )));
        }
        let name = ElementName::new(name).map_err(|e| {
            TextError::Rules(format!(
                "invalid unimportant item {item:?}: {} ({})",
                e.what, e.reason
            ))
        })?;
        Ok(UnimportantItem::Element(name.as_str().to_owned()))
    }

    /// The canonical word.
    pub fn as_str(&self) -> Cow<'_, str> {
        match self {
            UnimportantItem::LeadingWhitespace => Cow::Borrowed("leading-ws"),
            UnimportantItem::EmbeddedWhitespace => Cow::Borrowed("embedded-ws"),
            UnimportantItem::TrailingWhitespace => Cow::Borrowed("trailing-ws"),
            UnimportantItem::Whitespace => Cow::Borrowed("whitespace"),
            UnimportantItem::Case => Cow::Borrowed("case"),
            UnimportantItem::EverythingElse => Cow::Borrowed("everything-else"),
            UnimportantItem::Element(name) => {
                let is_word = WORDS.iter().any(|(w, _)| w.eq_ignore_ascii_case(name))
                    || name.eq_ignore_ascii_case(EVERYTHING_ELSE_NAME);
                let has_prefix = name
                    .get(..ELEMENT_PREFIX.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(ELEMENT_PREFIX));
                if is_word || has_prefix {
                    Cow::Owned(format!("{ELEMENT_PREFIX}{name}"))
                } else {
                    Cow::Borrowed(name)
                }
            }
        }
    }
}

impl fmt::Display for UnimportantItem {
    /// Writes [`UnimportantItem::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl TryFrom<String> for UnimportantItem {
    type Error = TextError;

    /// Exactly one item of the vocabulary (the serde form).
    fn try_from(word: String) -> Result<UnimportantItem, TextError> {
        let item = word.trim();
        if item.is_empty() || item.contains(',') {
            return Err(TextError::Rules(format!(
                "invalid unimportant item {word:?}: expected one word"
            )));
        }
        UnimportantItem::parse_one(item)
    }
}

impl From<UnimportantItem> for String {
    fn from(item: UnimportantItem) -> String {
        item.as_str().into_owned()
    }
}

/// Algorithm choice; `Auto` per D-36.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmChoice {
    /// Histogram for formats with a non-empty grammar ("source code") while its estimated cost fits the
    /// fallback budget, else Standard; Standard without a grammar (D-36 as amended in wave 3).
    #[default]
    Auto,
    /// Line n ↔ line n.
    Unaligned,
    /// Recursive best-anchor alignment with skew tolerance and line weights (§2.14.5).
    Standard,
    /// imara-diff's O(ND) diff (no closeness, D-17).
    Myers,
    /// Unique-line anchors, Myers in the gaps.
    Patience,
    /// imara-diff's Histogram.
    Histogram,
}

impl AlgorithmChoice {
    /// Every choice, in declaration order.
    pub const ALL: [AlgorithmChoice; 6] = [
        AlgorithmChoice::Auto,
        AlgorithmChoice::Unaligned,
        AlgorithmChoice::Standard,
        AlgorithmChoice::Myers,
        AlgorithmChoice::Patience,
        AlgorithmChoice::Histogram,
    ];

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            AlgorithmChoice::Auto => "auto",
            AlgorithmChoice::Unaligned => "unaligned",
            AlgorithmChoice::Standard => "standard",
            AlgorithmChoice::Myers => "myers",
            AlgorithmChoice::Patience => "patience",
            AlgorithmChoice::Histogram => "histogram",
        }
    }

    /// Inverse of [`AlgorithmChoice::as_str`] (ASCII-case-insensitive; `None` for unknown names).
    pub fn parse(name: &str) -> Option<AlgorithmChoice> {
        AlgorithmChoice::ALL
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(name.trim()))
    }

    /// The engine for `format` before the size check of D-36: `Auto` is Histogram when the format has a
    /// non-empty grammar, Standard otherwise. `compare` (D-36 as amended in wave 3) keeps Histogram for `Auto`
    /// only while the stripped middle's `|A|·|B|` fits `AlignOptions::fallback_budget_for(n, m)`, and uses the
    /// bounded Standard otherwise; `TextComparison::algorithm` reports the engine actually used.
    pub fn resolve(self, format: &CompiledFormat) -> Algorithm {
        match self {
            AlgorithmChoice::Auto => {
                if format.grammar().is_empty() {
                    Algorithm::Standard
                } else {
                    Algorithm::Histogram
                }
            }
            AlgorithmChoice::Unaligned => Algorithm::Unaligned,
            AlgorithmChoice::Standard => Algorithm::Standard,
            AlgorithmChoice::Myers => Algorithm::Myers,
            AlgorithmChoice::Patience => Algorithm::Patience,
            AlgorithmChoice::Histogram => Algorithm::Histogram,
        }
    }
}

impl fmt::Display for AlgorithmChoice {
    /// Writes [`AlgorithmChoice::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Alignment rules (TC-010, TC-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignmentRules {
    /// Engine. Default Auto.
    pub algorithm: AlgorithmChoice,
    /// Skew tolerance (lines). Default 300.
    pub skew_tolerance: u32,
    /// Closeness matching (D-17). Default true.
    pub closeness: bool,
    /// Closeness threshold (per mille, 0..=1000). Default 500.
    pub closeness_threshold_permille: u16,
    /// Never align differences. Default false.
    pub never_align_differences: bool,
}

impl Default for AlignmentRules {
    /// D-04.
    fn default() -> AlignmentRules {
        AlignmentRules {
            algorithm: AlgorithmChoice::Auto,
            skew_tolerance: 300,
            closeness: true,
            closeness_threshold_permille: 500,
            never_align_differences: false,
        }
    }
}

/// Overrides from CLI flags or IPC options (`None` = keep; vectors are appended). Applied after the
/// `--rules-file` TOML (§3.5). Each field is independent, so a lone `--skew 100` keeps `AlgorithmChoice::Auto`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TextRulesPatch {
    /// `--algorithm`.
    pub algorithm: Option<AlgorithmChoice>,
    /// `--skew`.
    pub skew_tolerance: Option<u32>,
    /// `--closeness` / `--no-closeness`.
    pub closeness: Option<bool>,
    /// `--closeness-threshold`.
    pub closeness_threshold_permille: Option<u16>,
    /// `--never-align`.
    pub never_align_differences: Option<bool>,
    /// `--compare-line-endings`.
    pub compare_line_endings: Option<bool>,
    /// `--orphans-important` / `--no-orphans-important`.
    pub orphans_always_important: Option<bool>,
    /// `--unimportant <list>` items, applied with `ImportanceRules::apply_unimportant` after the fields above.
    pub unimportant: Vec<UnimportantItem>,
    /// `--unimportant-text <text>` (plain) and `--unimportant-regex <re>` patterns, appended in order.
    pub unimportant_text: Vec<TextPattern>,
}

/// Keys of the rules file (unknown keys → warnings, D-31).
const TOP_KEYS: [&str; 3] = ["schema_version", "importance", "alignment"];
const IMPORTANCE_KEYS: [&str; 9] = [
    "elements",
    "leading_whitespace",
    "embedded_whitespace",
    "trailing_whitespace",
    "everything_else",
    "everything_else_case_sensitive",
    "unimportant",
    "orphans_always_important",
    "compare_line_endings",
];
const PATTERN_KEYS: [&str; 3] = ["pattern", "regex", "match_case"];
const ALIGNMENT_KEYS: [&str; 5] = [
    "algorithm",
    "skew_tolerance",
    "closeness",
    "closeness_threshold_permille",
    "never_align_differences",
];

impl TextRules {
    /// Parses a rules file (unknown keys → warnings; newer schema → error).
    ///
    /// Errors: TOML syntax and type errors are `TextError::Rules`; a `schema_version` above
    /// [`TEXT_RULES_SCHEMA_VERSION`] is `TextError::Schema` (checked before the shape, so a newer file never
    /// reports a type error). Unknown keys become `unknown_config_key` warnings (D-31). Values are not
    /// range-checked here: [`TextRules::validate`] does that.
    pub fn from_toml(text: &str) -> Result<(TextRules, Vec<Warning>), TextError> {
        let table: Table = toml::from_str(text)
            .map_err(|e| TextError::Rules(format!("TOML: {}", one_line(&e.to_string()))))?;
        if let Some(Value::Integer(found)) = table.get("schema_version")
            && *found >= 0
        {
            let found = u32::try_from(*found).unwrap_or(u32::MAX);
            check_schema_version(found, TEXT_RULES_SCHEMA_VERSION).map_err(TextError::Schema)?;
        }
        let rules: TextRules = toml::from_str(text)
            .map_err(|e| TextError::Rules(format!("TOML: {}", one_line(&e.to_string()))))?;
        let warnings = unknown_keys(&table)
            .into_iter()
            .map(|key| Warning {
                code: WarningCode::UnknownConfigKey,
                message: format!("rules: unknown key `{key}`"),
                spec: None,
            })
            .collect();
        Ok((rules, warnings))
    }

    /// Canonical TOML (round-trips: INV-RULES-1).
    pub fn to_toml(&self) -> String {
        match toml::to_string_pretty(self) {
            Ok(text) => text,
            // Every field is representable in TOML (string keys, no `None`), so this arm is unreachable; it
            // stays total instead of panicking (T9).
            Err(e) => format!("# cannot serialise rules: {e}\n"),
        }
    }

    /// Applies a patch.
    pub fn apply(&mut self, patch: &TextRulesPatch) {
        let a = &mut self.alignment;
        if let Some(v) = patch.algorithm {
            a.algorithm = v;
        }
        if let Some(v) = patch.skew_tolerance {
            a.skew_tolerance = v;
        }
        if let Some(v) = patch.closeness {
            a.closeness = v;
        }
        if let Some(v) = patch.closeness_threshold_permille {
            a.closeness_threshold_permille = v;
        }
        if let Some(v) = patch.never_align_differences {
            a.never_align_differences = v;
        }
        let i = &mut self.importance;
        if let Some(v) = patch.compare_line_endings {
            i.compare_line_endings = v;
        }
        if let Some(v) = patch.orphans_always_important {
            i.orphans_always_important = v;
        }
        i.apply_unimportant(&patch.unimportant);
        i.unimportant.extend(patch.unimportant_text.iter().cloned());
    }

    /// Validates value ranges and compiles `unimportant` patterns.
    ///
    /// Checks: `schema_version` not newer than supported (`TextError::Schema`); the closeness threshold in
    /// 0..=1000; every key of `importance.elements` a valid element name other than the reserved
    /// `Everything Else` (whose importance is `everything_else`); every pattern non-empty and compilable within
    /// `limits` (§2.11). Everything else is `TextError::Rules`.
    pub fn validate(&self, limits: &RegexLimits) -> Result<(), TextError> {
        check_schema_version(self.schema_version, TEXT_RULES_SCHEMA_VERSION)
            .map_err(TextError::Schema)?;
        if self.alignment.closeness_threshold_permille > 1000 {
            return Err(TextError::Rules(format!(
                "alignment.closeness_threshold_permille: {} is not in 0..=1000",
                self.alignment.closeness_threshold_permille
            )));
        }
        for name in self.importance.elements.keys() {
            if name == EVERYTHING_ELSE_NAME {
                return Err(TextError::Rules(format!(
                    "importance.elements: \"{EVERYTHING_ELSE_NAME}\" is reserved; set \
                     importance.everything_else instead"
                )));
            }
            match ElementName::new(name.as_str()) {
                Ok(valid) if valid.as_str() == name => {}
                Ok(_) => {
                    return Err(TextError::Rules(format!(
                        "importance.elements: element name {name:?} has surrounding whitespace"
                    )));
                }
                Err(e) => {
                    return Err(TextError::Rules(format!(
                        "importance.elements: invalid element name {name:?} ({})",
                        e.reason
                    )));
                }
            }
        }
        for (k, pattern) in self.importance.unimportant.iter().enumerate() {
            pattern.compile(limits).map_err(|message| {
                TextError::Rules(format!("importance.unimportant[{k}]: {message}"))
            })?;
        }
        Ok(())
    }

    /// Effective importance of element `id` of `format`: `importance.elements[name]` if present, else the
    /// format's `ElementInfo::important`; `ElementId::EVERYTHING_ELSE` → `importance.everything_else`.
    pub fn element_important(&self, format: &CompiledFormat, id: ElementId) -> bool {
        if id == ElementId::EVERYTHING_ELSE {
            return self.importance.everything_else;
        }
        let table = format.grammar().elements();
        self.importance
            .elements
            .get(table.name(id))
            .copied()
            .unwrap_or_else(|| table.default_important(id))
    }

    /// The identity fast-path predicate (D-53, §2.13.4): no unimportant patterns, every whitespace class and
    /// everything-else important and case-sensitive, and every element of `format` important and
    /// case-sensitive.
    pub fn is_identity(&self, format: &CompiledFormat) -> bool {
        let i = &self.importance;
        if !(i.unimportant.is_empty()
            && i.leading_whitespace
            && i.embedded_whitespace
            && i.trailing_whitespace
            && i.everything_else
            && i.everything_else_case_sensitive)
        {
            return false;
        }
        let table = format.grammar().elements();
        (1..table.len())
            .map(ElementId)
            .all(|id| self.element_important(format, id) && table.case_sensitive(id))
    }

    /// The engine for `format` before the size check of D-36: [`AlgorithmChoice::resolve`] of
    /// `alignment.algorithm` (see there for the `Auto` rule that `compare` applies on top).
    pub fn algorithm_for(&self, format: &CompiledFormat) -> Algorithm {
        self.alignment.algorithm.resolve(format)
    }
}

/// Dotted paths of the keys of a rules file that the model does not know.
fn unknown_keys(table: &Table) -> Vec<String> {
    let mut out = Vec::new();
    for (key, value) in table {
        if !TOP_KEYS.contains(&key.as_str()) {
            out.push(key.clone());
            continue;
        }
        match (key.as_str(), value) {
            ("importance", Value::Table(t)) => {
                for (k, v) in t {
                    if !IMPORTANCE_KEYS.contains(&k.as_str()) {
                        out.push(format!("importance.{k}"));
                    } else if let ("unimportant", Value::Array(items)) = (k.as_str(), v) {
                        for (n, item) in items.iter().enumerate() {
                            if let Value::Table(p) = item {
                                for pk in p.keys() {
                                    if !PATTERN_KEYS.contains(&pk.as_str()) {
                                        out.push(format!("importance.unimportant[{n}].{pk}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ("alignment", Value::Table(t)) => {
                for k in t.keys() {
                    if !ALIGNMENT_KEYS.contains(&k.as_str()) {
                        out.push(format!("alignment.{k}"));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Makes a multi-line parser message fit in one line.
fn one_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_formats::{FormatId, FormatRegistry};

    #[test]
    fn defaults_are_d04() {
        let r = TextRules::default();
        assert_eq!(r.schema_version, 1);
        let i = &r.importance;
        assert!(i.elements.is_empty());
        assert!(i.leading_whitespace && i.embedded_whitespace && i.trailing_whitespace);
        assert!(i.everything_else && i.everything_else_case_sensitive);
        assert!(i.unimportant.is_empty());
        assert!(i.orphans_always_important);
        assert!(!i.compare_line_endings);
        let a = r.alignment;
        assert_eq!(a.algorithm, AlgorithmChoice::Auto);
        assert_eq!(a.skew_tolerance, 300);
        assert!(a.closeness);
        assert_eq!(a.closeness_threshold_permille, 500);
        assert!(!a.never_align_differences);
    }

    #[test]
    fn parse_list_words_and_elements() {
        let items = UnimportantItem::parse_list(
            " leading-ws, EMBEDDED-WS ,trailing-ws,,whitespace,Case,everything-else, Comment ,element:case, ELEMENT:String",
        )
        .unwrap();
        assert_eq!(
            items,
            [
                UnimportantItem::LeadingWhitespace,
                UnimportantItem::EmbeddedWhitespace,
                UnimportantItem::TrailingWhitespace,
                UnimportantItem::Whitespace,
                UnimportantItem::Case,
                UnimportantItem::EverythingElse,
                UnimportantItem::Element("Comment".into()),
                UnimportantItem::Element("case".into()),
                UnimportantItem::Element("String".into()),
            ]
        );
        assert!(UnimportantItem::parse_list("").unwrap().is_empty());
        assert!(UnimportantItem::parse_list(" , ,").unwrap().is_empty());
        // Invalid names: control characters, too long, empty after the prefix.
        assert!(matches!(
            UnimportantItem::parse_list("bad\u{7}name"),
            Err(TextError::Rules(_))
        ));
        assert!(UnimportantItem::parse_list(&"x".repeat(65)).is_err());
        assert!(UnimportantItem::parse_list("element:").is_err());
        // Canonical words round-trip.
        for item in items {
            let word = item.as_str().into_owned();
            assert_eq!(
                UnimportantItem::parse_list(&word).unwrap(),
                std::slice::from_ref(&item)
            );
            let json = serde_json::to_string(&item).unwrap();
            assert_eq!(json, format!("\"{word}\""));
            assert_eq!(
                serde_json::from_str::<UnimportantItem>(&json).unwrap(),
                item
            );
        }
        assert_eq!(
            UnimportantItem::Element("case".into()).as_str(),
            "element:case"
        );
        assert_eq!(
            UnimportantItem::Element("element:x".into()).as_str(),
            "element:element:x"
        );
        assert!(serde_json::from_str::<UnimportantItem>("\"a,b\"").is_err());
    }

    /// The reserved name: the bare word means everything-else; as an element name it is rejected (it would
    /// otherwise silently do nothing).
    #[test]
    fn everything_else_is_a_word_not_an_element() {
        for word in ["Everything Else", "everything else", " EVERYTHING ELSE "] {
            assert_eq!(
                UnimportantItem::parse_list(word).unwrap(),
                [UnimportantItem::EverythingElse],
                "{word:?}"
            );
        }
        assert!(matches!(
            UnimportantItem::parse_list("element:Everything Else"),
            Err(TextError::Rules(_))
        ));
        // A user element spelled differently stays an element and round-trips with its prefix.
        let item = UnimportantItem::parse_list("element:everything else").unwrap();
        assert_eq!(item, [UnimportantItem::Element("everything else".into())]);
        assert_eq!(item[0].as_str(), "element:everything else");
        assert_eq!(
            UnimportantItem::parse_list(&item[0].as_str()).unwrap(),
            item
        );
        let mut r = TextRules::default();
        r.importance
            .elements
            .insert("Everything Else".into(), false);
        assert!(matches!(
            r.validate(&RegexLimits::default()),
            Err(TextError::Rules(m)) if m.contains("reserved")
        ));
    }

    #[test]
    fn apply_unimportant_sets_flags() {
        let mut i = ImportanceRules::default();
        i.apply_unimportant(&[UnimportantItem::Whitespace]);
        assert!(!i.leading_whitespace && !i.embedded_whitespace && !i.trailing_whitespace);
        assert!(i.everything_else);
        let mut i = ImportanceRules::default();
        i.apply_unimportant(&[
            UnimportantItem::LeadingWhitespace,
            UnimportantItem::Case,
            UnimportantItem::EverythingElse,
            UnimportantItem::Element("Comment".into()),
        ]);
        assert!(!i.leading_whitespace && i.embedded_whitespace && i.trailing_whitespace);
        assert!(!i.everything_else_case_sensitive && !i.everything_else);
        assert_eq!(i.elements.get("Comment"), Some(&false));
        let mut i = ImportanceRules::default();
        i.apply_unimportant(&[
            UnimportantItem::EmbeddedWhitespace,
            UnimportantItem::TrailingWhitespace,
        ]);
        assert!(i.leading_whitespace && !i.embedded_whitespace && !i.trailing_whitespace);
    }

    #[test]
    fn patches_apply_fields_then_items_then_patterns() {
        let mut r = TextRules::default();
        r.apply(&TextRulesPatch {
            skew_tolerance: Some(100),
            ..TextRulesPatch::default()
        });
        assert_eq!(r.alignment.algorithm, AlgorithmChoice::Auto);
        assert_eq!(r.alignment.skew_tolerance, 100);
        r.apply(&TextRulesPatch {
            algorithm: Some(AlgorithmChoice::Myers),
            closeness: Some(false),
            closeness_threshold_permille: Some(700),
            never_align_differences: Some(true),
            compare_line_endings: Some(true),
            orphans_always_important: Some(false),
            unimportant: vec![UnimportantItem::Case],
            unimportant_text: vec![TextPattern::plain("x"), TextPattern::regex("y+")],
            ..TextRulesPatch::default()
        });
        assert_eq!(r.alignment.algorithm, AlgorithmChoice::Myers);
        assert!(!r.alignment.closeness);
        assert_eq!(r.alignment.closeness_threshold_permille, 700);
        assert!(r.alignment.never_align_differences);
        assert!(r.importance.compare_line_endings);
        assert!(!r.importance.orphans_always_important);
        assert!(!r.importance.everything_else_case_sensitive);
        assert_eq!(r.importance.unimportant.len(), 2);
    }

    #[test]
    fn toml_round_trip_unknown_keys_and_schema() {
        let mut r = TextRules::default();
        r.importance.elements.insert("Comment".into(), false);
        r.importance.elements.insert("Weird \"name\"".into(), true);
        r.importance.unimportant.push(TextPattern {
            pattern: "\\$Id:[^$]*\\$".into(),
            regex: true,
            match_case: false,
        });
        r.alignment.algorithm = AlgorithmChoice::Patience;
        let text = r.to_toml();
        let (back, warnings) = TextRules::from_toml(&text).unwrap();
        assert_eq!(back, r);
        assert!(warnings.is_empty());

        let (r, warnings) = TextRules::from_toml(
            "schema_version = 1\nextra = 1\n[importance]\nleading_whitespace = false\nfoo = 2\n\
             [[importance.unimportant]]\npattern = \"x\"\nbar = true\n[alignment]\nskew_tolerance = 5\nbaz = 1\n",
        )
        .unwrap();
        assert!(!r.importance.leading_whitespace);
        assert_eq!(r.alignment.skew_tolerance, 5);
        assert!(r.importance.unimportant[0].match_case);
        let keys: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            keys,
            [
                "rules: unknown key `alignment.baz`",
                "rules: unknown key `extra`",
                "rules: unknown key `importance.foo`",
                "rules: unknown key `importance.unimportant[0].bar`",
            ]
        );
        assert!(
            warnings
                .iter()
                .all(|w| w.code == WarningCode::UnknownConfigKey)
        );
        // Missing keys take the D-04 defaults.
        assert_eq!(TextRules::from_toml("").unwrap().0, TextRules::default());
        // Newer schema: an error that classifies as unsupported, checked before the shape.
        let err = TextRules::from_toml("schema_version = 2\nimportance = 5\n").unwrap_err();
        assert!(matches!(err, TextError::Schema(_)), "{err}");
        let err = TextRules::from_toml("schema_version = 99999999999\n").unwrap_err();
        assert!(matches!(err, TextError::Schema(_)), "{err}");
        assert!(matches!(
            TextRules::from_toml("[importance\n"),
            Err(TextError::Rules(_))
        ));
        assert!(matches!(
            TextRules::from_toml("[alignment]\nalgorithm = \"fast\"\n"),
            Err(TextError::Rules(_))
        ));
    }

    #[test]
    fn validation() {
        let limits = RegexLimits::default();
        assert!(TextRules::default().validate(&limits).is_ok());
        let mut r = TextRules::default();
        r.alignment.closeness_threshold_permille = 1001;
        assert!(matches!(r.validate(&limits), Err(TextError::Rules(_))));
        let mut r = TextRules::default();
        r.importance.unimportant.push(TextPattern::regex("("));
        assert!(
            matches!(r.validate(&limits), Err(TextError::Rules(m)) if m.contains("unimportant[0]"))
        );
        let mut r = TextRules::default();
        r.importance.unimportant.push(TextPattern::plain(""));
        assert!(r.validate(&limits).is_err());
        let mut r = TextRules::default();
        r.importance
            .unimportant
            .push(TextPattern::plain("(literal"));
        assert!(r.validate(&limits).is_ok());
        let mut r = TextRules::default();
        r.importance.elements.insert("a,b".into(), false);
        assert!(r.validate(&limits).is_err());
        let mut r = TextRules::default();
        r.importance.elements.insert(" padded".into(), false);
        assert!(r.validate(&limits).is_err());
        let r = TextRules {
            schema_version: 2,
            ..TextRules::default()
        };
        assert!(matches!(r.validate(&limits), Err(TextError::Schema(_))));
    }

    #[test]
    fn identity_for_the_defaults_with_every_preset() {
        let registry = FormatRegistry::builtin().unwrap();
        let rules = TextRules::default();
        for entry in registry.entries() {
            let format = registry.compiled(entry.id()).unwrap();
            let table = format.grammar().elements();
            let expected = (1..table.len())
                .map(ElementId)
                .all(|id| table.default_important(id) && table.case_sensitive(id));
            assert_eq!(rules.is_identity(&format), expected, "{}", entry.id());
        }
        assert!(rules.is_identity(registry.everything_else()));
        // Known presets: C has a case-insensitive Number, Python an unimportant Whitespace element.
        let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
        assert!(!rules.is_identity(&c));
        let py = registry
            .compiled(&FormatId::new("python").unwrap())
            .unwrap();
        assert!(!rules.is_identity(&py));
        // Any opt-in breaks identity; overriding an unimportant element back to important restores it.
        let mut r = TextRules::default();
        r.importance.unimportant.push(TextPattern::plain("x"));
        assert!(!r.is_identity(registry.everything_else()));
        let mut r = TextRules::default();
        r.importance.everything_else_case_sensitive = false;
        assert!(!r.is_identity(registry.everything_else()));
        let bash = registry.compiled(&FormatId::new("bash").unwrap()).unwrap();
        let mut r = TextRules::default();
        let bash_identity = r.is_identity(&bash);
        r.importance.elements.insert("Comment".into(), false);
        if bash_identity {
            assert!(!r.is_identity(&bash));
        }
        // element_important: rules override the format's default by name; EE follows everything_else.
        let mut r = TextRules::default();
        let ws = py.grammar().elements().id("Whitespace").unwrap();
        assert!(!r.element_important(&py, ws));
        r.importance.elements.insert("Whitespace".into(), true);
        assert!(r.element_important(&py, ws));
        r.importance.everything_else = false;
        assert!(!r.element_important(&py, ElementId::EVERYTHING_ELSE));
    }

    #[test]
    fn algorithm_resolution_d36() {
        let registry = FormatRegistry::builtin().unwrap();
        let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
        assert_eq!(AlgorithmChoice::Auto.resolve(&c), Algorithm::Histogram);
        assert_eq!(
            AlgorithmChoice::Auto.resolve(registry.everything_else()),
            Algorithm::Standard
        );
        for choice in AlgorithmChoice::ALL {
            assert_eq!(AlgorithmChoice::parse(choice.as_str()), Some(choice));
            assert_eq!(
                serde_json::to_string(&choice).unwrap(),
                format!("\"{}\"", choice.as_str())
            );
        }
        assert_eq!(AlgorithmChoice::parse("nope"), None);
    }
}
