//! The rule tabs of the text sessions (M2 W11c; contract §1.11.25, SES-020, SES-022, TC-010…TC-022, FF-003; D-316,
//! D-317): **Alignment** (the engine, skew tolerance, closeness and its threshold, never-align), **Importance** (the
//! unimportant text list, the element checklist with each element's origin and *Reset to format*, the whitespace
//! classes, everything else and its case, orphans, line endings, *Edit Grammar…*) and **Format** (per side the format
//! and the encoding, *Same format for both sides*). Text Compare has the three, Text Edit the Format tab.
//!
//! Each tab writes only the keys it shows and only those edited (T-61): an untouched field keeps the session's value
//! exactly, so loading a session and writing without an edit is the identity (P-TABS-ID). Core checks the rules
//! (`Service::check_text_rules`), the format names (`format_list`) and the encodings (`TextEncoding::for_label`); a
//! refusal stays on its tab. *Reset to format* removes the element's entry from the rules: the scope's write turns that
//! into `"format"` where an inherited layer holds the element and into no entry otherwise (D-317, core's
//! `apply_scope`).

use std::any::Any;
use std::collections::BTreeMap;

use hikasa_formats::TextEncoding;
use hikasa_session::Service;
use hikasa_session::sessions::{SessionRules, SessionType, Specs};
use hikasa_text::{AlgorithmChoice, TextPattern, TextRules};

use crate::session_dialog::{SessionTab, TabExtras};

/// The Alignment tab's title.
pub const ALIGNMENT: &str = "Alignment";
/// The Importance tab's title (`Rules` opens the dialog on it).
pub const IMPORTANCE: &str = "Importance";
/// The Format tab's title.
pub const FORMAT: &str = "Format";

/// The engines in the order the tab offers them, with their captions.
pub const ENGINES: [(AlgorithmChoice, &str); 6] = [
    (AlgorithmChoice::Auto, "Auto"),
    (AlgorithmChoice::Standard, "Standard"),
    (AlgorithmChoice::Unaligned, "Unaligned"),
    (AlgorithmChoice::Myers, "Myers"),
    (AlgorithmChoice::Patience, "Patience"),
    (AlgorithmChoice::Histogram, "Histogram"),
];

/// The rule tabs of a session type: Alignment, Importance and Format for `text-compare`, Format for `text-edit`.
pub fn rule_tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    match kind {
        SessionType::TextCompare => vec![
            Box::new(AlignmentTab::default()),
            Box::new(ImportanceTab::default()),
            Box::new(FormatTab::default()),
        ],
        SessionType::TextEdit => vec![Box::new(FormatTab {
            one_side: true,
            ..FormatTab::default()
        })],
        _ => Vec::new(),
    }
}

macro_rules! tab_any {
    () => {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    };
}

/// The text rules of a session's rules.
fn text_of(rules: &SessionRules) -> Option<&TextRules> {
    match rules {
        SessionRules::Text(t) => Some(t),
        _ => None,
    }
}

/// `rules` with its text rules changed by `edit` (unchanged for a session that has none).
fn with_text(rules: &SessionRules, edit: impl FnOnce(&mut TextRules)) -> SessionRules {
    let mut out = rules.clone();
    if let SessionRules::Text(t) = &mut out {
        edit(t);
    }
    out
}

/// `value()` into `target` when the field was edited (`now` differs from the session's reading `read`, T-61).
fn put<F: PartialEq, T>(now: &F, read: &F, target: &mut T, value: impl FnOnce() -> T) {
    if now != read {
        *target = value();
    }
}

/// Core's check of text rules: its message, or nothing.
fn core_check(svc: &Service, rules: &SessionRules) -> Option<String> {
    let rules = text_of(rules)?;
    svc.check_text_rules(rules).err().map(|e| e.to_string())
}

// ---- Alignment ---------------------------------------------------------------------------------------------------

/// The Alignment tab's fields (TC-010, TC-011, TC-012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignmentFields {
    /// The engine.
    pub algorithm: AlgorithmChoice,
    /// Skew tolerance in lines, as typed (Standard only).
    pub skew: String,
    /// *Use closeness matching*.
    pub closeness: bool,
    /// The closeness threshold in ‰, as typed (0–1,000).
    pub threshold: String,
    /// *Never align differences*.
    pub never_align: bool,
}

impl Default for AlignmentFields {
    fn default() -> AlignmentFields {
        AlignmentFields::of(&TextRules::default())
    }
}

impl AlignmentFields {
    fn of(r: &TextRules) -> AlignmentFields {
        let a = &r.alignment;
        AlignmentFields {
            algorithm: a.algorithm,
            skew: a.skew_tolerance.to_string(),
            closeness: a.closeness,
            threshold: a.closeness_threshold_permille.to_string(),
            never_align: a.never_align_differences,
        }
    }

    /// Writes the edited fields; a number that does not read is the error.
    fn write(&self, r: &mut TextRules) -> Result<(), String> {
        let read = AlignmentFields::of(r);
        let skew: u32 = number(&self.skew, "The skew tolerance")?;
        let threshold: u16 = number(&self.threshold, "The closeness threshold")?;
        let a = &mut r.alignment;
        put(&self.algorithm, &read.algorithm, &mut a.algorithm, || {
            self.algorithm
        });
        put(&self.skew, &read.skew, &mut a.skew_tolerance, || skew);
        put(&self.closeness, &read.closeness, &mut a.closeness, || {
            self.closeness
        });
        put(
            &self.threshold,
            &read.threshold,
            &mut a.closeness_threshold_permille,
            || threshold,
        );
        put(
            &self.never_align,
            &read.never_align,
            &mut a.never_align_differences,
            || self.never_align,
        );
        Ok(())
    }
}

/// A whole number field (trimmed).
fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    let t = text.trim();
    t.parse()
        .map_err(|_| format!("{what} must be a whole number, not `{t}`."))
}

/// The Alignment tab.
#[derive(Debug, Clone, Default)]
pub struct AlignmentTab {
    /// The working copy.
    pub fields: AlignmentFields,
    original: AlignmentFields,
    /// The engine *Auto* chose at the view's last comparison (D-36), shown beside *Auto*.
    pub auto_engine: Option<String>,
    /// Core's message, or a number that does not read.
    pub error: Option<String>,
}

impl SessionTab for AlignmentTab {
    fn title(&self) -> String {
        ALIGNMENT.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        let mut ok = true;
        let rules = with_text(current, |t| ok = self.fields.write(t).is_ok());
        ok.then_some(rules)
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = text_of(rules) {
            self.fields = AlignmentFields::of(t);
            self.original = self.fields.clone();
        }
    }

    fn prepare(&mut self, extras: &TabExtras) {
        self.auto_engine.clone_from(&extras.text.auto_engine);
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let mut result = Ok(());
        let rules = with_text(current, |t| result = self.fields.write(t));
        self.error = match result {
            Err(e) => Some(e),
            Ok(()) => core_check(svc, &rules),
        };
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Importance --------------------------------------------------------------------------------------------------

/// The Importance tab's fields (TC-020, TC-021, TC-022).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportanceFields {
    /// *Unimportant text*, in order.
    pub unimportant: Vec<TextPattern>,
    /// The session's own element entries (an element without one follows its format, D-317).
    pub elements: BTreeMap<String, bool>,
    /// *Leading whitespace* is important.
    pub leading: bool,
    /// *Embedded whitespace* is important.
    pub embedded: bool,
    /// *Trailing whitespace* is important.
    pub trailing: bool,
    /// *Everything else* is important.
    pub everything_else: bool,
    /// Case changes in everything else are important.
    pub everything_else_case: bool,
    /// *Orphan lines are always important*.
    pub orphans: bool,
    /// *Compare line endings*.
    pub line_endings: bool,
}

impl Default for ImportanceFields {
    fn default() -> ImportanceFields {
        ImportanceFields::of(&TextRules::default())
    }
}

impl ImportanceFields {
    fn of(r: &TextRules) -> ImportanceFields {
        let i = &r.importance;
        ImportanceFields {
            unimportant: i.unimportant.clone(),
            elements: i.elements.clone(),
            leading: i.leading_whitespace,
            embedded: i.embedded_whitespace,
            trailing: i.trailing_whitespace,
            everything_else: i.everything_else,
            everything_else_case: i.everything_else_case_sensitive,
            orphans: i.orphans_always_important,
            line_endings: i.compare_line_endings,
        }
    }

    fn write(&self, r: &mut TextRules) {
        let read = ImportanceFields::of(r);
        let i = &mut r.importance;
        put(
            &self.unimportant,
            &read.unimportant,
            &mut i.unimportant,
            || self.unimportant.clone(),
        );
        // Element entries one by one: an untouched entry stays as the session holds it.
        for (name, value) in &self.elements {
            if read.elements.get(name) != Some(value) {
                i.elements.insert(name.clone(), *value);
            }
        }
        for name in read.elements.keys() {
            if !self.elements.contains_key(name) {
                i.elements.remove(name);
            }
        }
        put(
            &self.leading,
            &read.leading,
            &mut i.leading_whitespace,
            || self.leading,
        );
        put(
            &self.embedded,
            &read.embedded,
            &mut i.embedded_whitespace,
            || self.embedded,
        );
        put(
            &self.trailing,
            &read.trailing,
            &mut i.trailing_whitespace,
            || self.trailing,
        );
        put(
            &self.everything_else,
            &read.everything_else,
            &mut i.everything_else,
            || self.everything_else,
        );
        put(
            &self.everything_else_case,
            &read.everything_else_case,
            &mut i.everything_else_case_sensitive,
            || self.everything_else_case,
        );
        put(
            &self.orphans,
            &read.orphans,
            &mut i.orphans_always_important,
            || self.orphans,
        );
        put(
            &self.line_endings,
            &read.line_endings,
            &mut i.compare_line_endings,
            || self.line_endings,
        );
    }
}

/// Where an element's importance comes from (D-317).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// No entry: the format's default.
    Format,
    /// The session's entry equals the defaults' (inherited).
    Defaults,
    /// The session's own entry.
    Session,
}

impl Origin {
    /// The caption of the checklist's origin column.
    pub fn caption(self) -> &'static str {
        match self {
            Origin::Format => "format",
            Origin::Defaults => "defaults",
            Origin::Session => "this session",
        }
    }
}

/// One row of the element checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementRow {
    /// The element name.
    pub name: String,
    /// *Important*.
    pub important: bool,
    /// Where it comes from.
    pub origin: Origin,
}

/// The Importance tab.
#[derive(Debug, Clone)]
pub struct ImportanceTab {
    /// The working copy.
    pub fields: ImportanceFields,
    original: ImportanceFields,
    /// Every element of both panes' formats with its format's default importance (union by name, D-281).
    pub format_elements: Vec<(String, bool)>,
    /// The element entries of the defaults (the origin column; `None` for a defaults entry itself).
    pub defaults: Option<BTreeMap<String, bool>>,
    /// The pattern being typed (*Add*, or *Edit* of the selected one).
    pub pattern: TextPattern,
    /// The selected pattern of the list.
    pub selected: Option<usize>,
    /// *Edit Grammar…* was pressed: the format editor of this format opens (the window's).
    pub edit_grammar: Option<String>,
    /// The focus pane's format (what *Edit Grammar…* edits).
    pub focus_format: Option<String>,
    /// Core's message.
    pub error: Option<String>,
}

impl Default for ImportanceTab {
    fn default() -> ImportanceTab {
        ImportanceTab {
            fields: ImportanceFields::default(),
            original: ImportanceFields::default(),
            format_elements: Vec::new(),
            defaults: None,
            pattern: TextPattern::plain(""),
            selected: None,
            edit_grammar: None,
            focus_format: None,
            error: None,
        }
    }
}

impl ImportanceTab {
    /// The element checklist: the formats' elements, then the names only the rules hold, each with its importance and
    /// origin.
    pub fn rows(&self) -> Vec<ElementRow> {
        let mut names: Vec<(String, bool)> = self.format_elements.clone();
        for name in self.fields.elements.keys() {
            if !names.iter().any(|(n, _)| n == name) {
                names.push((name.clone(), true));
            }
        }
        names
            .into_iter()
            .map(|(name, default)| {
                let own = self.fields.elements.get(&name).copied();
                let origin = match own {
                    None => Origin::Format,
                    Some(v)
                        if self
                            .defaults
                            .as_ref()
                            .and_then(|d| d.get(&name))
                            .is_some_and(|d| *d == v) =>
                    {
                        Origin::Defaults
                    }
                    Some(_) => Origin::Session,
                };
                ElementRow {
                    important: own.unwrap_or(default),
                    name,
                    origin,
                }
            })
            .collect()
    }

    /// Ticks or unticks *Important* of an element: the session's own entry.
    pub fn set_important(&mut self, name: &str, important: bool) {
        self.fields.elements.insert(name.to_owned(), important);
    }

    /// *Reset to format* (D-317): the element follows its format again.
    pub fn reset_to_format(&mut self, name: &str) {
        self.fields.elements.remove(name);
    }

    /// *Add*: the typed pattern joins the list.
    pub fn add_pattern(&mut self) {
        if self.pattern.pattern.is_empty() {
            return;
        }
        self.fields.unimportant.push(self.pattern.clone());
        self.selected = Some(self.fields.unimportant.len() - 1);
        self.pattern = TextPattern::plain("");
    }

    /// *Edit*: the selected pattern takes the typed one.
    pub fn edit_pattern(&mut self) {
        if let Some(k) = self.selected.filter(|&k| k < self.fields.unimportant.len())
            && !self.pattern.pattern.is_empty()
        {
            self.fields.unimportant[k] = self.pattern.clone();
        }
    }

    /// Selects a pattern (its fields go to the editing row).
    pub fn select(&mut self, k: usize) {
        if let Some(p) = self.fields.unimportant.get(k) {
            self.pattern = p.clone();
            self.selected = Some(k);
        }
    }

    /// *Remove*.
    pub fn remove_pattern(&mut self) {
        if let Some(k) = self.selected.filter(|&k| k < self.fields.unimportant.len()) {
            self.fields.unimportant.remove(k);
            self.selected = None;
        }
    }

    /// *Move Up* (`-1`) or *Move Down* (`1`).
    pub fn move_pattern(&mut self, by: isize) {
        let Some(k) = self.selected.filter(|&k| k < self.fields.unimportant.len()) else {
            return;
        };
        let to = k as isize + by;
        if to < 0 || to as usize >= self.fields.unimportant.len() {
            return;
        }
        self.fields.unimportant.swap(k, to as usize);
        self.selected = Some(to as usize);
    }

    /// *Edit Grammar…*: the format editor of the focus pane's format.
    pub fn press_edit_grammar(&mut self) {
        self.edit_grammar.clone_from(&self.focus_format);
    }
}

impl SessionTab for ImportanceTab {
    fn title(&self) -> String {
        IMPORTANCE.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_text(current, |t| self.fields.write(t)))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = text_of(rules) {
            self.fields = ImportanceFields::of(t);
            self.original = self.fields.clone();
        }
    }

    fn prepare(&mut self, extras: &TabExtras) {
        self.format_elements.clone_from(&extras.text.elements);
        self.defaults = extras
            .text
            .defaults
            .as_ref()
            .and_then(text_of)
            .map(|r| r.importance.elements.clone());
        self.focus_format.clone_from(&extras.text.focus_format);
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        self.error = core_check(svc, &with_text(current, |t| self.fields.write(t)));
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Format ------------------------------------------------------------------------------------------------------

/// The Format tab's fields (FF-003, TC-001): `""` is *Detect*.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatFields {
    /// *Same format for both sides*.
    pub same: bool,
    /// The format of both sides (`specs.format`), shown while `same`.
    pub format: String,
    /// The left pane's own format.
    pub left: String,
    /// The right pane's own format.
    pub right: String,
    /// The left encoding.
    pub left_encoding: String,
    /// The right encoding.
    pub right_encoding: String,
}

impl FormatFields {
    fn of(s: &Specs) -> FormatFields {
        FormatFields {
            same: s.left_format.is_empty() && s.right_format.is_empty(),
            format: s.format.clone(),
            left: if s.left_format.is_empty() {
                s.format.clone()
            } else {
                s.left_format.clone()
            },
            right: if s.right_format.is_empty() {
                s.format.clone()
            } else {
                s.right_format.clone()
            },
            left_encoding: s.left_encoding.clone(),
            right_encoding: s.right_encoding.clone(),
        }
    }

    /// Writes the edited keys into the specs (this view only, D-82).
    fn write(&self, s: &mut Specs) {
        let read = FormatFields::of(s);
        if self.same {
            if self.same != read.same || self.format != read.format {
                s.format.clone_from(&self.format);
                s.left_format.clear();
                s.right_format.clear();
            }
        } else {
            put(&self.left, &read.left, &mut s.left_format, || {
                self.left.clone()
            });
            put(&self.right, &read.right, &mut s.right_format, || {
                self.right.clone()
            });
        }
        put(
            &self.left_encoding,
            &read.left_encoding,
            &mut s.left_encoding,
            || self.left_encoding.clone(),
        );
        put(
            &self.right_encoding,
            &read.right_encoding,
            &mut s.right_encoding,
            || self.right_encoding.clone(),
        );
    }
}

/// The Format tab.
#[derive(Debug, Clone, Default)]
pub struct FormatTab {
    /// The working copy.
    pub fields: FormatFields,
    original: FormatFields,
    /// A Text Edit view's one pane (the left fields only).
    pub one_side: bool,
    /// The names offered: every enabled text format (FF-003); *Detect* is `""`, *Everything Else* is in the list.
    pub formats: Vec<String>,
    /// Every format the registry knows: id, name (a name or an id resolves).
    known: Vec<(String, String)>,
    /// The specs are disabled (a defaults entry, D-82): the tab shows and writes nothing.
    pub disabled: bool,
    /// Why OK is off.
    pub error: Option<String>,
}

impl FormatTab {
    /// Fills the tab from the session's specs.
    pub fn load_specs(&mut self, specs: &Specs, disabled: bool) {
        self.fields = FormatFields::of(specs);
        self.original = self.fields.clone();
        self.disabled = disabled;
    }

    /// True when a field changed.
    pub fn edited(&self) -> bool {
        self.fields != self.original
    }

    /// The first format that does not resolve or encoding label that is no encoding.
    fn problem(&self) -> Option<String> {
        let names: Vec<&str> = if self.fields.same {
            vec![&self.fields.format]
        } else if self.one_side {
            vec![&self.fields.left]
        } else {
            vec![&self.fields.left, &self.fields.right]
        };
        for name in names.into_iter().filter(|n| !n.is_empty()) {
            let known = self
                .known
                .iter()
                .any(|(id, n)| id == name || n.eq_ignore_ascii_case(name));
            if !known && !self.known.is_empty() {
                return Some(format!("There is no format `{name}`."));
            }
        }
        let encodings: &[&String] = if self.one_side {
            &[&self.fields.left_encoding]
        } else {
            &[&self.fields.left_encoding, &self.fields.right_encoding]
        };
        encodings
            .iter()
            .find(|l| !l.is_empty() && TextEncoding::for_label(l).is_none())
            .map(|l| format!("`{l}` is not a text encoding."))
    }
}

impl SessionTab for FormatTab {
    fn title(&self) -> String {
        FORMAT.to_owned()
    }

    fn changes_rules(&self) -> bool {
        false
    }

    fn rules(&self, _current: &SessionRules) -> Option<SessionRules> {
        None
    }

    fn specs(&self, specs: &mut Specs) {
        if !self.disabled && self.edited() {
            self.fields.write(specs);
        }
    }

    fn prepare(&mut self, extras: &TabExtras) {
        self.formats = extras
            .text
            .text_formats
            .iter()
            .map(|(_, name)| name.clone())
            .collect();
        self.known = extras
            .formats
            .iter()
            .map(|(id, name, _)| (id.clone(), name.clone()))
            .collect();
    }

    fn check(&mut self, _svc: &Service, _current: &SessionRules) {
        self.error = if self.disabled { None } else { self.problem() };
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_registry_per_type() {
        let titles = |k| rule_tabs(k).iter().map(|t| t.title()).collect::<Vec<_>>();
        assert_eq!(
            titles(SessionType::TextCompare),
            [ALIGNMENT, IMPORTANCE, FORMAT]
        );
        assert_eq!(titles(SessionType::TextEdit), [FORMAT]);
        assert!(titles(SessionType::FolderCompare).is_empty());
    }

    #[test]
    fn the_checklist_and_reset_to_format() {
        let mut tab = ImportanceTab {
            format_elements: vec![("Comment".into(), true), ("Whitespace".into(), false)],
            defaults: Some(BTreeMap::from([("Comment".to_owned(), false)])),
            ..ImportanceTab::default()
        };
        tab.fields.elements =
            BTreeMap::from([("Comment".to_owned(), false), ("Extra".to_owned(), true)]);
        let rows = tab.rows();
        assert_eq!(
            rows.iter()
                .map(|r| (r.name.as_str(), r.important, r.origin))
                .collect::<Vec<_>>(),
            [
                ("Comment", false, Origin::Defaults),
                ("Whitespace", false, Origin::Format),
                ("Extra", true, Origin::Session),
            ]
        );
        tab.reset_to_format("Comment");
        let mut r = TextRules::default();
        r.importance.elements.insert("Comment".into(), false);
        r.importance.elements.insert("Extra".into(), true);
        tab.fields.write(&mut r);
        assert_eq!(
            r.importance.elements,
            BTreeMap::from([("Extra".to_owned(), true)])
        );
    }

    #[test]
    fn same_format_writes_format_and_clears_the_sides() {
        let specs = Specs {
            left_format: "c-cpp".into(),
            right_format: "java".into(),
            ..Specs::default()
        };
        let mut f = FormatFields::of(&specs);
        assert!(!f.same);
        let mut out = specs.clone();
        f.write(&mut out);
        assert_eq!(out, specs, "untouched: the identity");
        f.same = true;
        f.format = "rust".into();
        f.write(&mut out);
        assert_eq!(
            (
                out.format.as_str(),
                out.left_format.as_str(),
                out.right_format.as_str()
            ),
            ("rust", "", "")
        );
    }

    prop_compose! {
        fn text_rules()(
            algorithm in prop::sample::select(AlgorithmChoice::ALL.to_vec()),
            skew in any::<u32>(),
            closeness in any::<bool>(),
            threshold in 0u16..=1000,
            never in any::<bool>(),
            patterns in prop::collection::vec(("[a-c.*]{1,4}", any::<bool>(), any::<bool>()), 0..4),
            elements in prop::collection::btree_map("[A-D][a-z]{0,3}", any::<bool>(), 0..5),
            flags in prop::array::uniform7(any::<bool>()),
        ) -> TextRules {
            let mut r = TextRules::default();
            r.alignment.algorithm = algorithm;
            r.alignment.skew_tolerance = skew;
            r.alignment.closeness = closeness;
            r.alignment.closeness_threshold_permille = threshold;
            r.alignment.never_align_differences = never;
            r.importance.unimportant = patterns
                .into_iter()
                .map(|(pattern, regex, match_case)| TextPattern { pattern, regex, match_case })
                .collect();
            r.importance.elements = elements;
            let i = &mut r.importance;
            [i.leading_whitespace, i.embedded_whitespace, i.trailing_whitespace, i.everything_else,
             i.everything_else_case_sensitive, i.orphans_always_important, i.compare_line_endings] = flags;
            r
        }
    }

    prop_compose! {
        fn specs()(
            words in prop::array::uniform5(prop_oneof![Just(String::new()), "[a-z-]{1,6}"]),
        ) -> Specs {
            let [format, left_format, right_format, left_encoding, right_encoding] = words;
            Specs { format, left_format, right_format, left_encoding, right_encoding, ..Specs::default() }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// P-TABS-ID: loading a session into the tabs and writing without an edit is the identity, for every tab; an
        /// edit of one field changes that key only.
        #[test]
        fn p_tabs_id(r in text_rules(), s in specs(), base in text_rules()) {
            let rules = SessionRules::Text(r.clone());
            for mut tab in rule_tabs(SessionType::TextCompare) {
                tab.load(&rules);
                prop_assert!(!tab.changes_rules());
                if let Some(out) = tab.rules(&rules) {
                    prop_assert_eq!(&out, &rules);
                }
            }
            let mut f = FormatTab::default();
            f.load_specs(&s, false);
            let mut out = s.clone();
            SessionTab::specs(&f, &mut out);
            prop_assert_eq!(&out, &s);
            // Fields read from one session written over another: only what differs between the two moves.
            let a = AlignmentFields::of(&r);
            let mut w = base.clone();
            a.write(&mut w).unwrap();
            prop_assert_eq!(w.alignment, r.alignment);
            prop_assert_eq!(&w.importance, &base.importance);
            let i = ImportanceFields::of(&r);
            let mut w = base.clone();
            i.write(&mut w);
            prop_assert_eq!(&w.importance, &r.importance);
            prop_assert_eq!(w.alignment, base.alignment);
        }
    }
}
