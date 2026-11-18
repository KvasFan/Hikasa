//! Comparison options and their vocabulary (contract §5.1, §5.7, T-10…T-12): [`TableCompareOptions`],
//! [`SheetRules`], [`ColumnRule`], [`RulePatch`], [`ColumnRef`], [`Tolerance`] and the word parsers. Every
//! vocabulary word is parsed here (check 7: adapters never parse them).

use std::fmt;

use hikasa_diff::Algorithm;
use serde::{Deserialize, Serialize};

/// Normalizes a vocabulary word: ASCII lower case, `_` = `-`.
fn word(w: &str) -> String {
    w.trim().to_ascii_lowercase().replace('_', "-")
}

/// A reference to a column (T-12).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum ColumnRef {
    /// 1-based position (`3`, `#3`).
    Position(u32),
    /// A header name (`name:<text>`).
    Name(String),
    /// A header name, else column letters (`id`, `B`).
    Auto(String),
}

impl ColumnRef {
    /// Parses one reference (T-12). Empty text is refused.
    pub fn parse(text: &str) -> Result<ColumnRef, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("empty column reference".to_owned());
        }
        if let Some(name) = t.strip_prefix("name:") {
            if name.is_empty() {
                return Err("empty column name after `name:`".to_owned());
            }
            return Ok(ColumnRef::Name(name.to_owned()));
        }
        let digits = t.strip_prefix('#').unwrap_or(t);
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            let n: u32 = digits
                .parse()
                .map_err(|_| format!("column number `{t}` is too large"))?;
            if n == 0 {
                return Err("column numbers start at 1".to_owned());
            }
            return Ok(ColumnRef::Position(n));
        }
        if t.starts_with('#') {
            return Err(format!("`{t}` is not a column number"));
        }
        Ok(ColumnRef::Auto(t.to_owned()))
    }
}

impl fmt::Display for ColumnRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColumnRef::Position(n) => write!(f, "{n}"),
            ColumnRef::Name(n) => write!(f, "name:{n}"),
            ColumnRef::Auto(n) => f.write_str(n),
        }
    }
}

impl From<ColumnRef> for String {
    fn from(r: ColumnRef) -> String {
        r.to_string()
    }
}

impl TryFrom<String> for ColumnRef {
    type Error = String;
    fn try_from(s: String) -> Result<ColumnRef, String> {
        ColumnRef::parse(&s)
    }
}

/// Parses a comma-separated list of references; `\,` is a literal comma.
pub fn parse_column_list(text: &str) -> Result<Vec<ColumnRef>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                cur.push(',');
                chars.next();
            }
            ',' => {
                out.push(ColumnRef::parse(&cur)?);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    out.push(ColumnRef::parse(&cur)?);
    Ok(out)
}

/// A numeric tolerance (TB-012).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tolerance {
    /// `|a − b| ≤ v`.
    Absolute(f64),
    /// `|a − b| ≤ v / 100 · max(|a|, |b|)`.
    Relative(f64),
}

impl Tolerance {
    /// Parses `0.01` (absolute) or `0.5%` (relative); non-negative finite numbers only.
    pub fn parse(text: &str) -> Result<Tolerance, String> {
        let t = text.trim();
        let (num, rel) = match t.strip_suffix('%') {
            Some(n) => (n.trim(), true),
            None => (t, false),
        };
        let v: f64 = num
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| {
                format!("`{t}` is not a tolerance (a number ≥ 0, or a percentage like 0.5%)")
            })?;
        Ok(if rel {
            Tolerance::Relative(v)
        } else {
            Tolerance::Absolute(v)
        })
    }

    /// True when `a` and `b` are within the tolerance.
    pub fn within(&self, a: f64, b: f64) -> bool {
        let d = (a - b).abs();
        match *self {
            Tolerance::Absolute(v) => d <= v,
            Tolerance::Relative(p) => d <= p / 100.0 * a.abs().max(b.abs()),
        }
    }
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tolerance::Absolute(v) => write!(f, "{v}"),
            Tolerance::Relative(v) => write!(f, "{v}%"),
        }
    }
}

/// Parses a date tolerance into milliseconds: `500ms`, `30s`, `5m`, `2h`, `1d`, a bare number = seconds.
pub fn parse_date_tolerance(text: &str) -> Result<i64, String> {
    let t = text.trim().to_ascii_lowercase();
    let (num, unit) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1000.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60_000.0)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3_600_000.0)
    } else if let Some(n) = t.strip_suffix('d') {
        (n, 86_400_000.0)
    } else {
        (t.as_str(), 1000.0)
    };
    let v: f64 = num
        .trim()
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| {
            format!("`{text}` is not a date tolerance (a number with ms, s, m, h or d)")
        })?;
    let ms = v * unit;
    if ms > 1e15 {
        return Err(format!("date tolerance `{text}` is too large"));
    }
    Ok(ms.round() as i64)
}

/// Splits `[<cols>=]<value>` at its last `=` without reading either part: the columns as written (when present)
/// and the value as written. [`parse_column_value`] reads the two parts; an editor that must write an entry back
/// exactly as it was (the GUI's session tabs) keeps the parts as written.
pub fn split_column_value(text: &str) -> (Option<&str>, &str) {
    match text.rsplit_once('=') {
        Some((cols, value)) => (Some(cols), value),
        None => (None, text),
    }
}

/// Splits `[<cols>=]<value>` into its column list (when present) and the value (the last `=` separates).
pub fn parse_column_value(text: &str) -> Result<(Option<Vec<ColumnRef>>, String), String> {
    match split_column_value(text) {
        (Some(cols), value) => Ok((Some(parse_column_list(cols)?), value.trim().to_owned())),
        (None, value) => Ok((None, value.trim().to_owned())),
    }
}

/// Column type of a rule (TB-012 *Type*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    /// Typed comparison (T-6).
    #[default]
    General,
    /// Display texts compare (forced Text).
    Text,
    /// Every text cell that is a number compares as a number, leading zeros included (`007` = `7`; T-6, F2).
    Number,
}

impl ColumnType {
    /// Parses `general`, `text`, `number` (ASCII case-insensitive).
    pub fn parse(text: &str) -> Result<ColumnType, String> {
        match word(text).as_str() {
            "general" | "auto" => Ok(ColumnType::General),
            "text" => Ok(ColumnType::Text),
            "number" | "numeric" => Ok(ColumnType::Number),
            _ => Err(format!(
                "unknown column type `{text}` (expected general, text or number)"
            )),
        }
    }

    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ColumnType::General => "general",
            ColumnType::Text => "text",
            ColumnType::Number => "number",
        }
    }
}

/// Handling of one comparison column (TB-012).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnRule {
    /// General or Text.
    pub kind: ColumnType,
    /// Differences in the column are unimportant.
    pub unimportant: bool,
    /// Case differences are unimportant.
    pub ignore_case: bool,
    /// Whitespace differences (leading, trailing, runs) are unimportant.
    pub ignore_whitespace: bool,
    /// Numeric tolerance.
    pub numeric_tolerance: Option<Tolerance>,
    /// Date (and duration) tolerance in milliseconds.
    pub date_tolerance_ms: Option<i64>,
    /// Workbook numbers compare by their exact binary value; by default at 15 significant digits, the precision a
    /// workbook shows (`0.1 + 0.2` = `0.3`; F6).
    pub exact_numbers: bool,
}

/// A partial [`ColumnRule`]: the fields it sets replace the rule's (later patches win).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RulePatch {
    /// Sets `kind`.
    pub kind: Option<ColumnType>,
    /// Sets `unimportant`.
    pub unimportant: Option<bool>,
    /// Sets `ignore_case`.
    pub ignore_case: Option<bool>,
    /// Sets `ignore_whitespace`.
    pub ignore_whitespace: Option<bool>,
    /// Sets `numeric_tolerance` (`Some(None)` clears it).
    pub numeric_tolerance: Option<Option<Tolerance>>,
    /// Sets `date_tolerance_ms`.
    pub date_tolerance_ms: Option<Option<i64>>,
    /// Sets `exact_numbers`.
    pub exact_numbers: Option<bool>,
}

impl RulePatch {
    /// Applies the patch.
    pub fn apply(&self, rule: &mut ColumnRule) {
        if let Some(v) = self.kind {
            rule.kind = v;
        }
        if let Some(v) = self.unimportant {
            rule.unimportant = v;
        }
        if let Some(v) = self.ignore_case {
            rule.ignore_case = v;
        }
        if let Some(v) = self.ignore_whitespace {
            rule.ignore_whitespace = v;
        }
        if let Some(v) = self.numeric_tolerance {
            rule.numeric_tolerance = v;
        }
        if let Some(v) = self.date_tolerance_ms {
            rule.date_tolerance_ms = v;
        }
        if let Some(v) = self.exact_numbers {
            rule.exact_numbers = v;
        }
    }
}

/// Sheet alignment (TB-010, T-10).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetAlignment {
    /// Equal names, then case-insensitive names, then the rest by position (default).
    #[default]
    ByName,
    /// Pairs by name only, in the left side's order.
    LeftName,
    /// Pairs by name only, in the right side's order.
    RightName,
    /// Pairs by position.
    Position,
    /// Exactly these pairs.
    Custom(Vec<SheetPair>),
}

impl SheetAlignment {
    /// Parses `by-name`, `left-name`, `right-name`, `position`.
    pub fn parse(text: &str) -> Result<SheetAlignment, String> {
        match word(text).as_str() {
            "by-name" | "name" => Ok(SheetAlignment::ByName),
            "left-name" => Ok(SheetAlignment::LeftName),
            "right-name" => Ok(SheetAlignment::RightName),
            "position" | "unaligned" => Ok(SheetAlignment::Position),
            _ => Err(format!(
                "unknown sheet alignment `{text}` (expected by-name, left-name, right-name or position)"
            )),
        }
    }
}

/// A sheet named by 1-based position or by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetSelector {
    /// 1-based position.
    Position(u32),
    /// Name (exact, then case-insensitive).
    Name(String),
}

impl SheetSelector {
    /// A decimal is a position, anything else a name; `name:` forces a name.
    pub fn parse(text: &str) -> Result<SheetSelector, String> {
        let t = text.trim();
        if let Some(n) = t.strip_prefix("name:") {
            return Ok(SheetSelector::Name(n.to_owned()));
        }
        if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
            let n: u32 = t
                .parse()
                .map_err(|_| format!("sheet number `{t}` is too large"))?;
            if n == 0 {
                return Err("sheet numbers start at 1".to_owned());
            }
            return Ok(SheetSelector::Position(n));
        }
        if t.is_empty() {
            return Err("empty sheet reference".to_owned());
        }
        Ok(SheetSelector::Name(t.to_owned()))
    }

    /// Index of the sheet among `names`.
    pub fn resolve<'a>(&self, names: impl Iterator<Item = &'a str> + Clone) -> Option<usize> {
        match self {
            SheetSelector::Position(n) => {
                let i = *n as usize - 1;
                (i < names.clone().count()).then_some(i)
            }
            SheetSelector::Name(name) => names.clone().position(|n| n == name).or_else(|| {
                names
                    .clone()
                    .position(|n| hikasa_core::case::eq_folded(n, name))
            }),
        }
    }
}

impl fmt::Display for SheetSelector {
    /// As the user writes it: the number, the name (`name:` first when the name is a number, F11).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SheetSelector::Position(n) => write!(f, "{n}"),
            SheetSelector::Name(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                write!(f, "name:{n}")
            }
            SheetSelector::Name(n) => f.write_str(n),
        }
    }
}

/// One explicit sheet pair (a side may be absent: an orphan).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SheetPair {
    /// Left sheet.
    pub left: Option<SheetSelector>,
    /// Right sheet.
    pub right: Option<SheetSelector>,
}

impl SheetPair {
    /// Parses `<L>=<R>`, `<L>=`, `=<R>`, or `<S>` (the same sheet on both sides).
    pub fn parse(text: &str) -> Result<SheetPair, String> {
        let side = |s: &str| -> Result<Option<SheetSelector>, String> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                SheetSelector::parse(s).map(Some)
            }
        };
        let pair = match text.split_once('=') {
            Some((l, r)) => SheetPair {
                left: side(l)?,
                right: side(r)?,
            },
            None => SheetPair {
                left: side(text)?,
                right: side(text)?,
            },
        };
        if pair.left.is_none() && pair.right.is_none() {
            return Err("a sheet pair needs a left or a right sheet".to_owned());
        }
        Ok(pair)
    }
}

/// Column alignment (TB-011, T-11).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnAlignment {
    /// By name when both sides have header names, else by number (default).
    #[default]
    Auto,
    /// By number.
    Number,
    /// By name, in the left side's order.
    LeftName,
    /// By name, in the right side's order.
    RightName,
    /// These pairs first (references resolve per side), the remaining columns by number.
    Custom(Vec<(Option<ColumnRef>, Option<ColumnRef>)>),
}

impl ColumnAlignment {
    /// Parses `auto`, `number`, `left-name`, `right-name`.
    pub fn parse(text: &str) -> Result<ColumnAlignment, String> {
        match word(text).as_str() {
            "auto" => Ok(ColumnAlignment::Auto),
            "number" | "unaligned" | "position" => Ok(ColumnAlignment::Number),
            "left-name" | "name" => Ok(ColumnAlignment::LeftName),
            "right-name" => Ok(ColumnAlignment::RightName),
            _ => Err(format!(
                "unknown column alignment `{text}` (expected auto, number, left-name or right-name)"
            )),
        }
    }
}

/// Parses an explicit column pair `<L>=<R>` (`<L>=` and `=<R>` make orphans).
pub fn parse_column_pair(text: &str) -> Result<(Option<ColumnRef>, Option<ColumnRef>), String> {
    let (l, r) = text
        .split_once('=')
        .ok_or_else(|| format!("`{text}` is not a column pair (expected <left>=<right>)"))?;
    let side = |s: &str| -> Result<Option<ColumnRef>, String> {
        if s.trim().is_empty() {
            Ok(None)
        } else {
            ColumnRef::parse(s).map(Some)
        }
    };
    let pair = (side(l)?, side(r)?);
    if pair.0.is_none() && pair.1.is_none() {
        return Err("a column pair needs a left or a right column".to_owned());
    }
    Ok(pair)
}

/// Parses an alignment engine word (`unaligned`, `standard`, `myers`, `patience`, `histogram`).
pub fn parse_algorithm(text: &str) -> Result<Algorithm, String> {
    let w = word(text);
    Algorithm::ALL
        .into_iter()
        .find(|a| a.as_str() == w)
        .ok_or_else(|| {
            format!(
                "unknown alignment `{text}` (expected unaligned, standard, myers, patience or histogram)"
            )
        })
}

/// Row alignment of unkeyed rows (TB-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RowAlignment {
    /// Engine.
    pub algorithm: Algorithm,
    /// Skew tolerance in rows (Standard).
    pub skew: u32,
    /// Closeness threshold per mille (`None`: off).
    pub closeness: Option<u16>,
    /// Never pair differing rows.
    pub never_align: bool,
    /// Sort both sides by their full typed rows before aligning.
    pub sort_rows: bool,
}

impl Default for RowAlignment {
    /// Standard, skew 300, closeness 500 ‰.
    fn default() -> RowAlignment {
        RowAlignment {
            algorithm: Algorithm::Standard,
            skew: 300,
            closeness: Some(500),
            never_align: false,
            sort_rows: false,
        }
    }
}

/// Rules of one sheet pair (TB-011…TB-013).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetRules {
    /// Column alignment.
    pub columns: ColumnAlignment,
    /// Key columns, priority in order.
    pub keys: Vec<ColumnRef>,
    /// The default rule of every column.
    pub default_rule: ColumnRule,
    /// Per-column patches, applied in order on top of the default.
    pub column_rules: Vec<(ColumnRef, RulePatch)>,
    /// Unkeyed row alignment.
    pub rows: RowAlignment,
    /// Hidden columns (display only).
    pub hidden: Vec<ColumnRef>,
    /// Rows hidden in their workbook are left out of the comparison, as if absent from their side (F13).
    pub ignore_hidden_rows: bool,
    /// Columns hidden in their workbook are left out of the comparison, as if absent from their side.
    pub ignore_hidden_columns: bool,
    /// Rules given for every sheet (T-30): a column reference that names no column of a sheet is left out there with
    /// the warning `rule_not_applied` (keys that do not all resolve leave the sheet unkeyed), and only a reference
    /// that names nothing in every paired sheet with these rules is `InvalidInput`. Off: every reference must name a
    /// column of each sheet, except the ones of [`SheetRules::inherited`].
    pub lenient_references: bool,
    /// The references these rules inherited from the rules of every sheet (per-sheet rules start from them): they
    /// stay lenient when `lenient_references` is off (T-30; phase-2 review F1). Only the references written for the
    /// sheet are strict.
    pub inherited: InheritedReferences,
}

/// The references of a [`SheetRules`] inherited from the rules of every sheet (T-30): the first `column_rules`
/// patches, the first `hidden` references, and the keys when `keys` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InheritedReferences {
    /// The first this many entries of [`SheetRules::column_rules`].
    pub column_rules: usize,
    /// The first this many entries of [`SheetRules::hidden`].
    pub hidden: usize,
    /// [`SheetRules::keys`] are inherited.
    pub keys: bool,
}

/// Options of a comparison (contract §5.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TableCompareOptions {
    /// Sheet alignment.
    pub sheets: SheetAlignment,
    /// Rules of every sheet pair.
    pub default_sheet: SheetRules,
    /// Rules of particular pairs (matched by the left sheet, else the right sheet); the first match wins.
    pub sheet_rules: Vec<(SheetSelector, SheetRules)>,
    /// Differing formula texts are differences too (T-13).
    pub compare_formulas: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_refs() {
        assert_eq!(ColumnRef::parse("3"), Ok(ColumnRef::Position(3)));
        assert_eq!(ColumnRef::parse("#12"), Ok(ColumnRef::Position(12)));
        assert_eq!(
            ColumnRef::parse("name:12"),
            Ok(ColumnRef::Name("12".into()))
        );
        assert_eq!(ColumnRef::parse("id"), Ok(ColumnRef::Auto("id".into())));
        assert!(ColumnRef::parse("0").is_err());
        assert!(ColumnRef::parse("#x").is_err());
        assert!(ColumnRef::parse(" ").is_err());
        assert_eq!(
            parse_column_list(r"a\,b,2"),
            Ok(vec![ColumnRef::Auto("a,b".into()), ColumnRef::Position(2)])
        );
        for r in [
            ColumnRef::Position(4),
            ColumnRef::Name("x".into()),
            ColumnRef::Auto("B".into()),
        ] {
            assert_eq!(ColumnRef::parse(&r.to_string()), Ok(r));
        }
    }

    #[test]
    fn tolerances_and_words() {
        assert_eq!(Tolerance::parse("0.5%"), Ok(Tolerance::Relative(0.5)));
        assert_eq!(Tolerance::parse("0.01"), Ok(Tolerance::Absolute(0.01)));
        assert!(Tolerance::parse("-1").is_err());
        assert!(Tolerance::parse("x").is_err());
        assert!(Tolerance::Absolute(0.1).within(1.0, 1.1 - 1e-12));
        assert!(Tolerance::Relative(1.0).within(100.0, 101.0));
        assert!(!Tolerance::Relative(1.0).within(100.0, 102.0));
        assert_eq!(parse_date_tolerance("30s"), Ok(30_000));
        assert_eq!(parse_date_tolerance("2"), Ok(2_000));
        assert_eq!(parse_date_tolerance("1d"), Ok(86_400_000));
        assert_eq!(parse_date_tolerance("250ms"), Ok(250));
        assert!(parse_date_tolerance("1y").is_err());
        assert_eq!(
            parse_column_value("price,qty=0.5%"),
            Ok((
                Some(vec![
                    ColumnRef::Auto("price".into()),
                    ColumnRef::Auto("qty".into())
                ]),
                "0.5%".to_owned()
            ))
        );
        assert_eq!(parse_column_value("0.1"), Ok((None, "0.1".to_owned())));
        assert_eq!(
            SheetAlignment::parse("Left_Name"),
            Ok(SheetAlignment::LeftName)
        );
        assert!(SheetAlignment::parse("x").is_err());
        assert_eq!(
            ColumnAlignment::parse("NUMBER"),
            Ok(ColumnAlignment::Number)
        );
        assert_eq!(parse_algorithm("Patience"), Ok(Algorithm::Patience));
        assert!(parse_algorithm("auto").is_err());
        assert_eq!(
            SheetPair::parse("Data=2"),
            Ok(SheetPair {
                left: Some(SheetSelector::Name("Data".into())),
                right: Some(SheetSelector::Position(2)),
            })
        );
        assert_eq!(SheetPair::parse("=x").map(|p| p.left), Ok(None));
        assert!(SheetPair::parse("=").is_err());
        assert_eq!(
            parse_column_pair("id=key"),
            Ok((
                Some(ColumnRef::Auto("id".into())),
                Some(ColumnRef::Auto("key".into()))
            ))
        );
        let names = ["One", "two"];
        assert_eq!(
            SheetSelector::Name("TWO".into()).resolve(names.iter().copied()),
            Some(1)
        );
        assert_eq!(
            SheetSelector::Position(3).resolve(names.iter().copied()),
            None
        );
    }
}
