//! Validation and compilation of a [`FileFormat`]: [`CompiledFormat`], [`CompiledGrammar`], [`ElementId`] and
//! [`ElementTable`] (§1.4.4, D-41).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use hikasa_core::mask::{CaseSensitivity, MaskList, MaskOptions, MaskTarget};
use serde::{Deserialize, Serialize};

use crate::convert::{ConversionVars, build_argv};
use crate::encoding::TextEncoding;
use crate::error::FormatError;
use crate::grammar::ItemKind;
use crate::model::{
    Conversion, ElementInfo, ElementName, EncodingSetting, FileFormat, FormatId, FormatKind,
    TextSettings,
};
use crate::regex_engine::{FirstChars, Pattern, RegexLimits, first_chars};
use crate::tokenize::scan::{CompiledItem, ItemMatcher, ListMatcher, Matcher, WordClass};
use crate::weights::CompiledWeights;

/// The display name of the implicit format and of `ElementId(0)`.
pub const EVERYTHING_ELSE: &str = "Everything Else";

/// Index of an element in a compiled format: 0 = "Everything Else"; then first use in `grammar`; then unused
/// declared elements by name (D-41, INV-FMT-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(pub u16);

impl ElementId {
    /// Text not covered by any grammar item.
    pub const EVERYTHING_ELSE: ElementId = ElementId(0);
}

/// Element names and case flags of a compiled format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementTable {
    names: Vec<ElementName>,
    case_sensitive: Vec<bool>,
    important: Vec<bool>,
}

impl ElementTable {
    fn everything_else_only() -> ElementTable {
        ElementTable {
            names: vec![ElementName::everything_else()],
            case_sensitive: vec![true],
            important: vec![true],
        }
    }

    /// Name of `id` ("Everything Else" for 0; empty for an id outside the table).
    pub fn name(&self, id: ElementId) -> &str {
        self.names
            .get(usize::from(id.0))
            .map_or("", ElementName::as_str)
    }

    /// Case sensitivity of `id` (id 0: true; the rules decide case for everything-else text).
    pub fn case_sensitive(&self, id: ElementId) -> bool {
        self.case_sensitive
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(true)
    }

    /// Default importance of `id` from `ElementInfo::important` (id 0: true; the rules decide).
    pub fn default_important(&self, id: ElementId) -> bool {
        self.important
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(true)
    }

    /// Id by name.
    pub fn id(&self, name: &str) -> Option<ElementId> {
        self.names
            .iter()
            .position(|n| n.as_str() == name)
            .and_then(|k| u16::try_from(k).ok())
            .map(ElementId)
    }

    /// Number of ids (including 0).
    pub fn len(&self) -> u16 {
        u16::try_from(self.names.len()).unwrap_or(u16::MAX)
    }

    /// Always false: id 0 ("Everything Else") exists in every table.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Every id with its name, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (ElementId, &str)> + '_ {
        self.names
            .iter()
            .enumerate()
            .map(|(k, n)| (ElementId(u16::try_from(k).unwrap_or(u16::MAX)), n.as_str()))
    }
}

/// Candidate items per first character: index `b` (0..128) for an ASCII byte, 128 for any non-ASCII char.
#[derive(Debug, Clone)]
pub(crate) struct Prefilter {
    /// Flattened candidate lists: `items[ranges[c].0..ranges[c].1]`, items in grammar order.
    items: Vec<u16>,
    /// Lists for positions > 0 (classes 0..=128), then for position 0 (classes 129..=257).
    ranges: Vec<(u32, u32)>,
    /// `true` when no item can start at an ASCII byte at a position > 0 (fast everything-else skip).
    pub(crate) skip_mid: [bool; 128],
}

impl Prefilter {
    fn build(first: &[FirstChars]) -> Prefilter {
        let mut items = Vec::new();
        let mut ranges = Vec::with_capacity(258);
        for start in [false, true] {
            for class in 0..=128usize {
                let from = items.len();
                for (k, fc) in first.iter().enumerate() {
                    let admits = if class < 128 {
                        fc.ascii & (1u128 << class) != 0
                    } else {
                        fc.non_ascii
                    };
                    if admits || (start && fc.at_line_start) {
                        items.push(u16::try_from(k).unwrap_or(u16::MAX));
                    }
                }
                ranges.push((from as u32, items.len() as u32));
            }
        }
        let mut skip_mid = [false; 128];
        for (b, skip) in skip_mid.iter_mut().enumerate() {
            *skip = ranges[b].0 == ranges[b].1;
        }
        Prefilter {
            items,
            ranges,
            skip_mid,
        }
    }

    /// Candidate items for a char class (`0..128` ASCII byte, 128 non-ASCII) at a position.
    #[inline]
    pub(crate) fn candidates(&self, class: usize, at_line_start: bool) -> &[u16] {
        let index = class.min(128) + if at_line_start { 129 } else { 0 };
        match self.ranges.get(index) {
            Some(&(a, b)) => self.items.get(a as usize..b as usize).unwrap_or(&[]),
            None => &[],
        }
    }
}

/// The compiled grammar (matchers + first-char prefilter).
#[derive(Debug, Clone)]
pub struct CompiledGrammar {
    pub(crate) items: Vec<CompiledItem>,
    pub(crate) elements: ElementTable,
    pub(crate) prefilter: Prefilter,
    pub(crate) word: WordClass,
    /// Number of regex matchers (each owns one next-match cache slot in a tokenizer).
    pub(crate) regex_slots: usize,
    pub(crate) has_multiline: bool,
    pub(crate) has_columns: bool,
    /// The field boundaries: the 0-based start columns (> 0) of the Columns items, sorted and unique. No
    /// token of another item crosses one (§2.10.2).
    pub(crate) fences: Vec<u32>,
}

impl CompiledGrammar {
    fn empty() -> CompiledGrammar {
        CompiledGrammar {
            items: Vec::new(),
            elements: ElementTable::everything_else_only(),
            prefilter: Prefilter::build(&[]),
            word: WordClass::new(""),
            regex_slots: 0,
            has_multiline: false,
            has_columns: false,
            fences: Vec::new(),
        }
    }

    /// True when there are no items (only "Everything Else").
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Element table.
    pub fn elements(&self) -> &ElementTable {
        &self.elements
    }

    /// True when some item can leave a line in a non-`Normal` state (multi-line Delimited, or Lines with
    /// `next_lines > 0`). When false, every line starts in `Normal` and no start states are stored.
    pub fn has_multiline(&self) -> bool {
        self.has_multiline
    }

    /// Number of grammar items.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

/// A validated, compiled format. Immutable and `Send + Sync`; shared via `Arc`.
#[derive(Debug, Clone)]
pub struct CompiledFormat {
    format: Option<Arc<FileFormat>>,
    mask: Option<MaskList>,
    grammar: CompiledGrammar,
    weights: CompiledWeights,
    text: TextSettings,
    encoding: EncodingSetting,
    conversion: Conversion,
}

/// Collects compile errors with the format id.
struct Ctx<'a> {
    id: &'a str,
    limits: &'a RegexLimits,
}

impl Ctx<'_> {
    fn invalid(&self, field: impl Into<String>, reason: impl Into<String>) -> FormatError {
        FormatError::Invalid {
            id: self.id.to_owned(),
            field: field.into(),
            reason: reason.into(),
        }
    }

    fn pattern(&self, source: &str, ci: bool, location: String) -> Result<Pattern, FormatError> {
        Pattern::compile(source, ci, self.limits).map_err(|source| FormatError::Pattern {
            id: self.id.to_owned(),
            location,
            source,
        })
    }

    fn matcher(
        &self,
        text: &str,
        regex: bool,
        match_case: bool,
        location: String,
        slots: &mut usize,
    ) -> Result<(Matcher, FirstChars), FormatError> {
        if text.is_empty() {
            return Err(self.invalid(location, "must not be empty"));
        }
        let ci = !match_case;
        if regex {
            let pattern = self.pattern(text, ci, location)?;
            let first = first_chars(text, ci).unwrap_or(FirstChars::ANY);
            let slot = *slots;
            *slots += 1;
            Ok((Matcher::Regex { pattern, slot }, first))
        } else {
            Ok((
                Matcher::Literal {
                    text: text.into(),
                    ci,
                },
                FirstChars::of_literal(text, ci),
            ))
        }
    }
}

/// Parses the format mask (names only, case-insensitive, D-10).
pub(crate) fn compile_mask(id: &str, mask: &str) -> Result<MaskList, FormatError> {
    let list = MaskList::parse(
        mask,
        MaskOptions {
            case: CaseSensitivity::Insensitive,
        },
    )
    .map_err(|source| FormatError::Mask {
        id: id.to_owned(),
        source,
    })?;
    if let Some(bad) = list
        .masks()
        .iter()
        .find(|m| !m.is_simple() || m.target() == MaskTarget::Folders)
    {
        return Err(FormatError::Invalid {
            id: id.to_owned(),
            field: "mask".into(),
            reason: format!(
                "format masks match file names only; {:?} contains a separator",
                bad.source()
            ),
        });
    }
    Ok(list)
}

fn check_label(label: &str) -> Result<(), FormatError> {
    match TextEncoding::for_label(label) {
        Some(_) => Ok(()),
        None => Err(FormatError::UnknownEncoding {
            label: label.to_owned(),
        }),
    }
}

impl CompiledFormat {
    /// Validates and compiles (every regex compiled; columns/priority/tab ranges checked).
    pub fn compile(
        format: Arc<FileFormat>,
        limits: &RegexLimits,
    ) -> Result<CompiledFormat, FormatError> {
        let ctx = Ctx {
            id: format.id.as_str(),
            limits,
        };
        let text = &format.text;
        if !(1..=32).contains(&text.tab_size) {
            return Err(ctx.invalid("text.tab_size", "must be in 1..=32"));
        }
        if text.word_chars.chars().any(char::is_whitespace) {
            return Err(ctx.invalid("text.word_chars", "must not contain whitespace"));
        }
        match &format.encoding {
            EncodingSetting::Fixed { label } => check_label(label)?,
            EncodingSetting::Auto {
                fallback: Some(label),
            } => check_label(label)?,
            EncodingSetting::Auto { fallback: None } => {}
        }
        if let Conversion::External(ext) = &format.conversion {
            let vars = ConversionVars {
                source: "s".into(),
                target: "t".into(),
                original: "o".into(),
            };
            build_argv(&ext.load, &vars)
                .map_err(|e| ctx.invalid("conversion.load", e.to_string()))?;
            if let Some(save) = &ext.save {
                build_argv(save, &vars)
                    .map_err(|e| ctx.invalid("conversion.save", e.to_string()))?;
            }
        }
        if let Some(table) = &format.table {
            if matches!(table.delimiter, '\r' | '\n') {
                return Err(ctx.invalid("table.delimiter", "must not be a line break"));
            }
            if table.qualifier == Some(table.delimiter) {
                return Err(ctx.invalid("table.qualifier", "must differ from the delimiter"));
            }
        }
        let mask = compile_mask(ctx.id, &format.mask)?;
        let grammar = compile_grammar(&ctx, &format)?;
        let weights = CompiledWeights::compile(&format.line_weights, |k, w| {
            if !(1..=4).contains(&w.priority) {
                return Err(ctx.invalid(format!("line_weight[{k}].priority"), "must be in 1..=4"));
            }
            if w.pattern.is_empty() {
                return Err(ctx.invalid(format!("line_weight[{k}].pattern"), "must not be empty"));
            }
            if w.regex {
                ctx.pattern(
                    &w.pattern,
                    !w.match_case,
                    format!("line_weight[{k}].pattern"),
                )
                .map(Some)
            } else {
                Ok(None)
            }
        })?;
        Ok(CompiledFormat {
            mask: Some(mask),
            grammar,
            weights,
            text: format.text.clone(),
            encoding: format.encoding.clone(),
            conversion: format.conversion.clone(),
            format: Some(format),
        })
    }

    /// The implicit "Everything Else" text format (id `None`, no grammar, Auto encoding).
    pub fn everything_else() -> CompiledFormat {
        CompiledFormat {
            format: None,
            mask: None,
            grammar: CompiledGrammar::empty(),
            weights: CompiledWeights::default(),
            text: TextSettings::default(),
            encoding: EncodingSetting::Auto { fallback: None },
            conversion: Conversion::None,
        }
    }

    /// Source format (`None` for Everything Else).
    pub fn format(&self) -> Option<&FileFormat> {
        self.format.as_deref()
    }

    /// Id (`None` for Everything Else).
    pub fn id(&self) -> Option<&FormatId> {
        self.format.as_ref().map(|f| &f.id)
    }

    /// Display name ("Everything Else" for the implicit format).
    pub fn name(&self) -> &str {
        self.format.as_ref().map_or(EVERYTHING_ELSE, |f| &f.name)
    }

    /// Kind (Text for Everything Else).
    pub fn kind(&self) -> FormatKind {
        self.format.as_ref().map_or(FormatKind::Text, |f| f.kind)
    }

    /// Grammar.
    pub fn grammar(&self) -> &CompiledGrammar {
        &self.grammar
    }

    /// Text settings.
    pub fn text(&self) -> &TextSettings {
        &self.text
    }

    /// Encoding setting.
    pub fn encoding(&self) -> &EncodingSetting {
        &self.encoding
    }

    /// Conversion.
    pub fn conversion(&self) -> &Conversion {
        &self.conversion
    }

    /// The compiled name mask (case-insensitive, names only; `None` for Everything Else).
    pub fn mask(&self) -> Option<&MaskList> {
        self.mask.as_ref()
    }

    /// Weight (0..=4) of a raw line: the maximum priority among matching line weights (0 = none). A weight
    /// regex that hits the fancy-regex backtracking limit counts as "not found"; use
    /// [`CompiledFormat::line_weight_with_errors`] to count those hits (warning `regex_runtime_error`).
    pub fn line_weight(&self, line: &str) -> u8 {
        self.weights.weight(line).0
    }

    /// [`CompiledFormat::line_weight`] with the number of runtime-limit hits of the weight regexes on this line
    /// (each counts as "not found"; callers add them to `regex_errors`, §2.10.7, §5.5 rule 3).
    pub fn line_weight_with_errors(&self, line: &str) -> (u8, u32) {
        self.weights.weight(line)
    }

    /// True when the format declares line weights.
    pub fn has_line_weights(&self) -> bool {
        !self.weights.is_empty()
    }
}

fn compile_grammar(ctx: &Ctx<'_>, format: &FileFormat) -> Result<CompiledGrammar, FormatError> {
    if format.grammar.len() > usize::from(u16::MAX) {
        return Err(ctx.invalid("grammar", "more than 65535 items"));
    }
    // D-41: 0 = Everything Else, then first use in `grammar`, then unused declared names (BTreeMap order).
    let mut names: Vec<ElementName> = vec![ElementName::everything_else()];
    let mut index: HashMap<String, u16> = HashMap::new();
    let mut id_of = |name: &ElementName| -> Result<u16, FormatError> {
        if name.as_str() == EVERYTHING_ELSE {
            return Err(ctx.invalid(
                format!("element {:?}", name.as_str()),
                "the name \"Everything Else\" is reserved",
            ));
        }
        if let Some(&id) = index.get(name.as_str()) {
            return Ok(id);
        }
        let id = u16::try_from(names.len())
            .map_err(|_| ctx.invalid("elements", "more than 65535 elements"))?;
        names.push(name.clone());
        index.insert(name.as_str().to_owned(), id);
        Ok(id)
    };
    let mut item_ids = Vec::with_capacity(format.grammar.len());
    for item in &format.grammar {
        item_ids.push(id_of(&item.element)?);
    }
    for name in format.elements.keys() {
        id_of(name)?;
    }
    let info = |name: &ElementName| -> ElementInfo {
        format.elements.get(name).copied().unwrap_or_default()
    };
    let case_sensitive = names
        .iter()
        .enumerate()
        .map(|(k, n)| k == 0 || info(n).case_sensitive)
        .collect();
    let important = names
        .iter()
        .enumerate()
        .map(|(k, n)| k == 0 || info(n).important)
        .collect();
    let elements = ElementTable {
        names,
        case_sensitive,
        important,
    };

    let word = WordClass::new(&format.text.word_chars);
    let mut slots = 0usize;
    let mut items = Vec::with_capacity(format.grammar.len());
    let mut first = Vec::with_capacity(format.grammar.len());
    for (k, (item, &id)) in format.grammar.iter().zip(&item_ids).enumerate() {
        let at = |field: &str| format!("grammar[{k}].{field}");
        let (matcher, fc, multiline) = match &item.kind {
            ItemKind::Basic {
                text,
                regex,
                match_case,
            } => {
                let (m, fc) = ctx.matcher(text, *regex, *match_case, at("text"), &mut slots)?;
                (ItemMatcher::Basic(m), fc, false)
            }
            ItemKind::Delimited {
                from,
                to,
                stop_at_eol,
                escape,
                regex,
                match_case,
            } => {
                let (from_m, fc) =
                    ctx.matcher(from, *regex, *match_case, at("from"), &mut slots)?;
                let to_m = match to {
                    Some(to) => Some(
                        ctx.matcher(to, *regex, *match_case, at("to"), &mut slots)?
                            .0,
                    ),
                    None => None,
                };
                let multiline = to.is_some() && !stop_at_eol;
                (
                    ItemMatcher::Delimited {
                        from: from_m,
                        to: to_m,
                        escape: *escape,
                    },
                    fc,
                    multiline,
                )
            }
            ItemKind::List {
                tokens,
                regex,
                match_case,
            } => {
                if tokens.is_empty() {
                    return Err(ctx.invalid(at("tokens"), "must not be empty"));
                }
                let mut list = ListMatcher::new(!match_case);
                let mut fc = FirstChars::default();
                for (t, token) in tokens.iter().enumerate() {
                    let location = format!("grammar[{k}].tokens[{t}]");
                    if token.is_empty() {
                        return Err(ctx.invalid(location, "must not be empty"));
                    }
                    if !regex && token.chars().all(|c| word.is_word(c)) {
                        list.add_word(token);
                        fc.union(FirstChars::of_literal(token, !match_case));
                    } else {
                        let (m, tfc) =
                            ctx.matcher(token, *regex, *match_case, location, &mut slots)?;
                        list.add_other(m, tfc);
                        fc.union(tfc);
                    }
                }
                (ItemMatcher::List(list), fc, false)
            }
            ItemKind::Columns {
                from_column,
                to_column,
            } => {
                if *from_column == 0 {
                    return Err(ctx.invalid(at("from_column"), "columns are 1-based"));
                }
                if let Some(to) = to_column
                    && to < from_column
                {
                    return Err(ctx.invalid(at("to_column"), "must be ≥ from_column"));
                }
                (
                    ItemMatcher::Columns {
                        from: from_column - 1,
                        to: *to_column,
                    },
                    FirstChars::ANY,
                    false,
                )
            }
            ItemKind::Lines {
                heading,
                regex,
                match_case,
                next_lines,
                or_line_one,
            } => {
                let heading_m = if heading.is_empty() {
                    if !or_line_one {
                        return Err(ctx.invalid(
                            at("heading"),
                            "must not be empty unless or_line_one is set",
                        ));
                    }
                    None
                } else {
                    Some(
                        ctx.matcher(heading, *regex, *match_case, at("heading"), &mut slots)?
                            .0,
                    )
                };
                (
                    ItemMatcher::Lines {
                        heading: heading_m,
                        next_lines: *next_lines,
                        or_line_one: *or_line_one,
                    },
                    FirstChars {
                        ascii: 0,
                        non_ascii: false,
                        at_line_start: true,
                    },
                    *next_lines > 0,
                )
            }
        };
        items.push(CompiledItem {
            element: ElementId(id),
            matcher,
            multiline,
        });
        first.push(fc);
    }
    let has_multiline = items.iter().any(|i| i.multiline);
    let has_columns = items
        .iter()
        .any(|i| matches!(i.matcher, ItemMatcher::Columns { .. }));
    let fences: BTreeSet<u32> = items
        .iter()
        .filter_map(|i| match i.matcher {
            ItemMatcher::Columns { from, .. } if from > 0 => Some(from),
            _ => None,
        })
        .collect();
    Ok(CompiledGrammar {
        prefilter: Prefilter::build(&first),
        items,
        elements,
        word,
        regex_slots: slots,
        has_multiline,
        has_columns,
        fences: fences.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::GrammarItem;
    use crate::toml_io::parse_format;

    fn compile(text: &str) -> Result<CompiledFormat, FormatError> {
        let (f, _) = parse_format(text, "t").unwrap();
        CompiledFormat::compile(Arc::new(f), &RegexLimits::default())
    }

    const HEAD: &str = "schema_version = 1\nid = \"t\"\nname = \"T\"\nkind = \"text\"\n";

    #[test]
    fn element_ids_follow_first_use() {
        let text = format!(
            "{HEAD}[elements]\nAlpha = {{ important = false }}\nZed = {{ case_sensitive = false }}\n\
             Beta = {{}}\nUnused2 = {{}}\nUnused1 = {{}}\n\
             [[grammar]]\nelement = \"Zed\"\nkind = \"basic\"\ntext = \"z\"\n\
             [[grammar]]\nelement = \"Alpha\"\nkind = \"basic\"\ntext = \"a\"\n\
             [[grammar]]\nelement = \"Zed\"\nkind = \"basic\"\ntext = \"y\"\n\
             [[grammar]]\nelement = \"Undeclared\"\nkind = \"basic\"\ntext = \"u\"\n"
        );
        let f = compile(&text).unwrap();
        let e = f.grammar().elements();
        let names: Vec<_> = e.iter().map(|(_, n)| n).collect();
        assert_eq!(
            names,
            [
                "Everything Else",
                "Zed",
                "Alpha",
                "Undeclared",
                "Beta",
                "Unused1",
                "Unused2"
            ]
        );
        assert_eq!(e.id("Alpha"), Some(ElementId(2)));
        assert_eq!(e.id("nope"), None);
        assert!(!e.case_sensitive(ElementId(1)));
        assert!(e.case_sensitive(ElementId(2)));
        assert!(!e.default_important(ElementId(2)));
        assert!(e.default_important(ElementId(0)));
        assert!(e.default_important(ElementId(3)));
        assert_eq!(e.len(), 7);
        assert_eq!(e.name(ElementId(99)), "");
        assert_eq!(e.name(ElementId::EVERYTHING_ELSE), "Everything Else");
    }

    #[test]
    fn validation_errors() {
        let bad = |extra: &str| compile(&format!("{HEAD}{extra}")).unwrap_err().to_string();
        assert_eq!(
            bad("[text]\ntab_size = 0\n"),
            "format t: text.tab_size: must be in 1..=32"
        );
        assert_eq!(
            bad(
                "[[grammar]]\nelement = \"C\"\nkind = \"columns\"\nfrom_column = 5\nto_column = 2\n"
            ),
            "format t: grammar[0].to_column: must be ≥ from_column"
        );
        assert_eq!(
            bad("[[grammar]]\nelement = \"C\"\nkind = \"columns\"\nfrom_column = 0\n"),
            "format t: grammar[0].from_column: columns are 1-based"
        );
        assert_eq!(
            bad("[[line_weight]]\npattern = \"x\"\npriority = 5\n"),
            "format t: line_weight[0].priority: must be in 1..=4"
        );
        assert!(
            bad("[[grammar]]\nelement = \"C\"\nkind = \"basic\"\ntext = \"(\"\nregex = true\n")
                .starts_with("format t: grammar[0].text: pattern \"(\"")
        );
        assert_eq!(
            bad("mask = \"src/*.c\"\n"),
            "format t: mask: format masks match file names only; \"src/*.c\" contains a separator"
        );
        assert!(bad("mask = \"a[b\"\n").starts_with("format t: mask: mask \"a[b\""));
        assert_eq!(
            bad("[encoding]\nmode = \"fixed\"\nlabel = \"cp437\"\n"),
            "unknown encoding \"cp437\""
        );
        assert_eq!(
            bad("[[grammar]]\nelement = \"Everything Else\"\nkind = \"basic\"\ntext = \"x\"\n"),
            "format t: element \"Everything Else\": the name \"Everything Else\" is reserved"
        );
        assert_eq!(
            bad("[[grammar]]\nelement = \"K\"\nkind = \"list\"\ntokens = []\n"),
            "format t: grammar[0].tokens: must not be empty"
        );
        assert_eq!(
            bad("[[grammar]]\nelement = \"K\"\nkind = \"list\"\ntokens = [\"a\", \"\"]\n"),
            "format t: grammar[0].tokens[1]: must not be empty"
        );
        assert_eq!(
            bad("[[grammar]]\nelement = \"H\"\nkind = \"lines\"\nheading = \"\"\n"),
            "format t: grammar[0].heading: must not be empty unless or_line_one is set"
        );
        assert!(
            bad("[conversion]\nmethod = \"external\"\nload = 'x \"%s'\n")
                .starts_with("format t: conversion.load: command template:")
        );
        assert_eq!(
            bad("[table]\ndelimiter = \",\"\nqualifier = \",\"\n"),
            "format t: table.qualifier: must differ from the delimiter"
        );
        assert_eq!(
            bad("[text]\nword_chars = \"- \"\n"),
            "format t: text.word_chars: must not contain whitespace"
        );
    }

    #[test]
    fn everything_else_format() {
        let ee = CompiledFormat::everything_else();
        assert_eq!(ee.id(), None);
        assert_eq!(ee.name(), "Everything Else");
        assert_eq!(ee.kind(), FormatKind::Text);
        assert!(ee.grammar().is_empty());
        assert!(!ee.grammar().has_multiline());
        assert_eq!(ee.grammar().elements().len(), 1);
        assert_eq!(ee.line_weight("anything"), 0);
        assert!(ee.mask().is_none());
        assert!(ee.format().is_none());
    }

    #[test]
    fn multiline_flags() {
        let item = |kind: ItemKind| GrammarItem {
            element: ElementName::new("E").unwrap(),
            kind,
        };
        let mut f = FileFormat::new(FormatId::new("m").unwrap(), "M", FormatKind::Text);
        f.grammar.push(item(ItemKind::Delimited {
            from: "\"".into(),
            to: Some("\"".into()),
            stop_at_eol: true,
            escape: None,
            regex: false,
            match_case: true,
        }));
        let c = CompiledFormat::compile(Arc::new(f.clone()), &RegexLimits::default()).unwrap();
        assert!(!c.grammar().has_multiline());
        f.grammar.push(item(ItemKind::Lines {
            heading: "x".into(),
            regex: false,
            match_case: true,
            next_lines: 1,
            or_line_one: false,
        }));
        let c = CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap();
        assert!(c.grammar().has_multiline());
        assert_eq!(c.grammar().item_count(), 2);
    }
}
