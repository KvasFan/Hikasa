//! The rule tabs of a Table Compare session (TB-010…TB-013; m2-table-contract §6.6, T-35, T-58): **Sheets** (the
//! sheet alignment, custom pairs with Move Up/Down and Tidy, compare formulas), **Columns** (the column alignment,
//! custom column pairs, and the per-column rules over `ColumnRule` with the default row as *Edit Default*), **Rows**
//! (keys, the alignment engine, skew, closeness, never-align, sort, hidden rows and columns) and **Parsing** (the
//! parser words of TB-001…TB-003). Each tab edits the words of `TableSessionRules` (the session keeps words, T-35);
//! `Service::check_table_words` checks them as `table compare` would, and OK recompares the view through the
//! session's reopen path (a `JobKind::TableCompare` job, cancellable). The words are written as typed: the GUI parses
//! none (check 7). Per-sheet rules (`[[rules.sheet]]`) are kept as they are.

use std::any::Any;

use hikasa_session::Service;
use hikasa_session::api::{
    SheetRuleWords, TableCompareRequest, TableFormatWords, TableSessionRules,
};
use hikasa_session::sessions::{SessionRules, SessionType};

use crate::session_dialog::SessionTab;

/// The Sheets tab's title (`Rules` opens the dialog on it).
pub const SHEETS: &str = "Sheets";
/// The Columns tab's title.
pub const COLUMNS: &str = "Columns";
/// The Rows tab's title.
pub const ROWS: &str = "Rows";
/// The Parsing tab's title.
pub const PARSING: &str = "Parsing";
/// The help topic of the tabs.
pub const TOPIC: &str = "dialog-session-table";

/// The sheet alignment words with their captions (`""`: the default, by name).
pub const SHEET_MODES: [(&str, &str); 5] = [
    ("", "Default (by name)"),
    ("by-name", "By name, then by position"),
    ("left-name", "By the left sheets' names"),
    ("right-name", "By the right sheets' names"),
    ("position", "By position"),
];

/// The column alignment words with their captions.
pub const COLUMN_MODES: [(&str, &str); 6] = [
    ("", "Default (automatic)"),
    (
        "auto",
        "By name when both sides have header names, else by number",
    ),
    ("number", "By number"),
    ("left-name", "By the left header names"),
    ("right-name", "By the right header names"),
    ("custom", "Custom pairs (below), the others by number"),
];

/// The row alignment engines with their captions.
pub const ALGORITHMS: [(&str, &str); 6] = [
    ("", "Default (standard)"),
    ("standard", "Standard"),
    ("myers", "Myers"),
    ("patience", "Patience"),
    ("histogram", "Histogram"),
    ("unaligned", "Unaligned (row by row)"),
];

/// The rule tabs of a session type: the four table tabs for `table-compare`.
pub fn rule_tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    if kind != SessionType::TableCompare {
        return Vec::new();
    }
    vec![
        Box::new(SheetsTab::default()),
        Box::new(ColumnsTab::default()),
        Box::new(RowsTab::default()),
        Box::new(ParsingTab::default()),
    ]
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

/// The table words of a session's rules.
fn words_of(rules: &SessionRules) -> Option<&TableSessionRules> {
    match rules {
        SessionRules::Table(t) => Some(t),
        _ => None,
    }
}

/// `rules` with its table words changed by `edit` (unchanged for a session that has none).
fn with_table(rules: &SessionRules, edit: impl FnOnce(&mut TableSessionRules)) -> SessionRules {
    let mut out = rules.clone();
    if let SessionRules::Table(t) = &mut out {
        edit(t);
    }
    out
}

/// The core check of a tab's words: `Service::check_table_words`'s message, or nothing.
fn core_check(svc: &Service, rules: &SessionRules) -> Option<String> {
    let words = words_of(rules)?;
    let request = TableCompareRequest {
        parse: words.parse.clone(),
        rules: words.rules.clone(),
        ..TableCompareRequest::default()
    };
    svc.check_table_words(&request).err().map(|e| e.to_string())
}

/// `""` → `None`, else the trimmed text.
fn word(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// The non-empty lines of a list field, trimmed.
fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A list's entries as the lines of its field.
fn joined(list: &[String]) -> String {
    list.join("\n")
}

// ---- Sheets ------------------------------------------------------------------------------------------------------

/// The Sheets tab's fields (TB-010).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SheetsFields {
    /// The alignment word (`""`: the default).
    pub mode: String,
    /// Custom pairs, one `<left>=<right>` (or `<left>=`, `=<right>`) a line; given, they are the only pairs.
    pub pairs: Vec<String>,
    /// TB-013 *compare formulas* (T-13).
    pub formulas: bool,
}

/// The Sheets tab.
#[derive(Debug, Clone, Default)]
pub struct SheetsTab {
    /// The working copy.
    pub fields: SheetsFields,
    original: SheetsFields,
    /// The pair selected (Move Up/Down).
    pub selected: Option<usize>,
    /// The new pair being typed.
    pub new_pair: String,
    /// Core's message.
    pub error: Option<String>,
}

impl SheetsTab {
    /// Adds the typed pair.
    pub fn add_pair(&mut self) {
        if let Some(pair) = word(&self.new_pair) {
            self.fields.pairs.push(pair);
            self.selected = Some(self.fields.pairs.len() - 1);
            self.new_pair.clear();
        }
    }

    /// Removes the selected pair.
    pub fn remove_pair(&mut self) {
        if let Some(k) = self.selected.filter(|&k| k < self.fields.pairs.len()) {
            self.fields.pairs.remove(k);
            self.selected = None;
        }
    }

    /// Moves the selected pair up (`-1`) or down (`1`).
    pub fn move_pair(&mut self, by: isize) {
        let Some(k) = self.selected.filter(|&k| k < self.fields.pairs.len()) else {
            return;
        };
        let to = k as isize + by;
        if to < 0 || to as usize >= self.fields.pairs.len() {
            return;
        }
        self.fields.pairs.swap(k, to as usize);
        self.selected = Some(to as usize);
    }

    /// Tidy: trims the pairs, drops empty and repeated ones.
    pub fn tidy(&mut self) {
        let mut seen = Vec::new();
        for pair in self.fields.pairs.drain(..) {
            let pair = pair.trim().to_owned();
            if !pair.is_empty() && !seen.contains(&pair) {
                seen.push(pair);
            }
        }
        self.fields.pairs = seen;
        self.selected = None;
    }

    fn write(&self, t: &mut TableSessionRules) {
        t.rules.sheets = word(&self.fields.mode);
        t.rules.sheet_pairs = self.fields.pairs.clone();
        t.rules.formulas = self.fields.formulas;
    }
}

impl SessionTab for SheetsTab {
    fn title(&self) -> String {
        SHEETS.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_table(current, |t| self.write(t)))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = words_of(rules) {
            self.fields = SheetsFields {
                mode: t.rules.sheets.clone().unwrap_or_default(),
                pairs: t.rules.sheet_pairs.clone(),
                formulas: t.rules.formulas,
            };
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        self.error = core_check(svc, &with_table(current, |t| self.write(t)));
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Columns -----------------------------------------------------------------------------------------------------

/// One line of the column rules (TB-012): a column reference as typed (`""`: every column, *Edit Default*) and its
/// rule.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColumnRuleRow {
    /// The columns (a reference or a comma-separated list, T-12); `""` for the default row.
    pub columns: String,
    /// Unimportant column.
    pub unimportant: bool,
    /// Compared as text.
    pub text: bool,
    /// Compared as numbers (leading zeros parsed, T-24).
    pub number: bool,
    /// *Character case* is unimportant.
    pub ignore_case: bool,
    /// *Whitespace* is unimportant.
    pub ignore_whitespace: bool,
    /// Workbook numbers by their binary value (T-25).
    pub exact_numbers: bool,
    /// Hidden in the views and reports.
    pub hide: bool,
    /// Numeric tolerance (`0.01`, `0.5%`; `""`: none).
    pub tolerance: String,
    /// Date tolerance (`30s`, `5m`, `2h`, `1d`; `""`: none).
    pub date_tolerance: String,
}

/// The Columns tab's fields (TB-011, TB-012).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColumnsFields {
    /// The alignment word (`""`: the default).
    pub mode: String,
    /// Custom column pairs, one `<left>=<right>` a line.
    pub pairs: String,
    /// The default rule (every column).
    pub default: ColumnRuleRow,
    /// The rules of named columns, in order.
    pub rows: Vec<ColumnRuleRow>,
}

impl ColumnsFields {
    fn of(w: &SheetRuleWords) -> ColumnsFields {
        let mut out = ColumnsFields {
            mode: w.columns.clone().unwrap_or_default(),
            pairs: joined(&w.align_columns),
            ..ColumnsFields::default()
        };
        // The row of `columns` (`""`: the default), created on first use.
        fn row<'a>(out: &'a mut ColumnsFields, columns: &str) -> &'a mut ColumnRuleRow {
            if columns.trim().is_empty() {
                return &mut out.default;
            }
            let k = match out.rows.iter().position(|r| r.columns == columns) {
                Some(k) => k,
                None => {
                    out.rows.push(ColumnRuleRow {
                        columns: columns.to_owned(),
                        ..ColumnRuleRow::default()
                    });
                    out.rows.len() - 1
                }
            };
            &mut out.rows[k]
        }
        for c in &w.unimportant {
            row(&mut out, c).unimportant = true;
        }
        for c in &w.text {
            row(&mut out, c).text = true;
        }
        for c in &w.number {
            row(&mut out, c).number = true;
        }
        for c in &w.ignore_case {
            row(&mut out, c).ignore_case = true;
        }
        for c in &w.ignore_whitespace {
            row(&mut out, c).ignore_whitespace = true;
        }
        for c in &w.exact_numbers {
            row(&mut out, c).exact_numbers = true;
        }
        for c in &w.hide {
            row(&mut out, c).hide = true;
        }
        // `[<cols>=]<value>`: the value after the last `=` (a value never holds one).
        for entry in &w.tolerance {
            let (cols, value) = entry.rsplit_once('=').unwrap_or(("", entry));
            row(&mut out, cols).tolerance = value.to_owned();
        }
        for entry in &w.date_tolerance {
            let (cols, value) = entry.rsplit_once('=').unwrap_or(("", entry));
            row(&mut out, cols).date_tolerance = value.to_owned();
        }
        out
    }

    fn write(&self, w: &mut SheetRuleWords) {
        w.columns = word(&self.mode);
        w.align_columns = lines(&self.pairs);
        let named: Vec<&ColumnRuleRow> = self
            .rows
            .iter()
            .filter(|r| !r.columns.trim().is_empty())
            .collect();
        let list = |pick: fn(&ColumnRuleRow) -> bool| -> Vec<String> {
            named
                .iter()
                .filter(|r| pick(r))
                .map(|r| r.columns.trim().to_owned())
                .collect()
        };
        // A flag of every column is the empty value (`--ignore-case` without columns).
        let every = |on: bool, mut v: Vec<String>| {
            if on {
                v.insert(0, String::new());
            }
            v
        };
        w.unimportant = list(|r| r.unimportant);
        w.text = list(|r| r.text);
        w.number = list(|r| r.number);
        w.hide = list(|r| r.hide);
        w.ignore_case = every(self.default.ignore_case, list(|r| r.ignore_case));
        w.ignore_whitespace = every(
            self.default.ignore_whitespace,
            list(|r| r.ignore_whitespace),
        );
        w.exact_numbers = every(self.default.exact_numbers, list(|r| r.exact_numbers));
        let valued = |default: &str, pick: fn(&ColumnRuleRow) -> &str| -> Vec<String> {
            let mut v: Vec<String> = word(default).into_iter().collect();
            v.extend(named.iter().filter_map(|r| {
                word(pick(r)).map(|value| format!("{}={value}", r.columns.trim()))
            }));
            v
        };
        w.tolerance = valued(&self.default.tolerance, |r| &r.tolerance);
        w.date_tolerance = valued(&self.default.date_tolerance, |r| &r.date_tolerance);
    }
}

/// The Columns tab.
#[derive(Debug, Clone, Default)]
pub struct ColumnsTab {
    /// The working copy.
    pub fields: ColumnsFields,
    original: ColumnsFields,
    /// The rule row selected.
    pub selected: Option<usize>,
    /// The columns of the next rule row, as typed.
    pub new_columns: String,
    /// Core's message.
    pub error: Option<String>,
}

impl ColumnsTab {
    /// Adds a rule row for the typed columns.
    pub fn add_row(&mut self) {
        let Some(columns) = word(&self.new_columns) else {
            return;
        };
        self.new_columns.clear();
        self.fields.rows.push(ColumnRuleRow {
            columns,
            ..ColumnRuleRow::default()
        });
        self.selected = Some(self.fields.rows.len() - 1);
    }

    /// Removes the selected rule row.
    pub fn remove_row(&mut self) {
        if let Some(k) = self.selected.filter(|&k| k < self.fields.rows.len()) {
            self.fields.rows.remove(k);
            self.selected = None;
        }
    }
}

impl SessionTab for ColumnsTab {
    fn title(&self) -> String {
        COLUMNS.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_table(current, |t| {
            self.fields.write(&mut t.rules.rules)
        }))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = words_of(rules) {
            self.fields = ColumnsFields::of(&t.rules.rules);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let rules = with_table(current, |t| self.fields.write(&mut t.rules.rules));
        self.error = core_check(svc, &rules);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Rows --------------------------------------------------------------------------------------------------------

/// The Rows tab's fields (TB-013).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowsFields {
    /// Key columns in priority order (a comma-separated list as typed; `""`: no keys).
    pub keys: String,
    /// The engine word (`""`: the default).
    pub algorithm: String,
    /// Skew tolerance (`""`: the default).
    pub skew: String,
    /// Closeness matching: `None` the default, on, off.
    pub closeness: Option<bool>,
    /// The closeness threshold in ‰ (`""`: the default).
    pub threshold: String,
    /// *Never align differences*.
    pub never_align: bool,
    /// *Sort rows before alignment*.
    pub sort_rows: bool,
    /// Hidden rows are left out (T-29).
    pub ignore_hidden_rows: bool,
    /// Hidden columns are left out (T-29).
    pub ignore_hidden_columns: bool,
}

impl RowsFields {
    fn of(w: &SheetRuleWords) -> RowsFields {
        RowsFields {
            keys: w.keys.join(","),
            algorithm: w.algorithm.clone().unwrap_or_default(),
            skew: w.skew.map(|n| n.to_string()).unwrap_or_default(),
            closeness: w.closeness,
            threshold: w
                .closeness_threshold
                .map(|n| n.to_string())
                .unwrap_or_default(),
            never_align: w.never_align,
            sort_rows: w.sort_rows,
            ignore_hidden_rows: w.ignore_hidden_rows,
            ignore_hidden_columns: w.ignore_hidden_columns,
        }
    }

    /// Writes the fields; a number field that is not a number is the error.
    fn write(&self, w: &mut SheetRuleWords) -> Result<(), String> {
        w.keys = word(&self.keys).into_iter().collect();
        w.algorithm = word(&self.algorithm);
        w.skew = number(&self.skew, "The skew tolerance")?;
        w.closeness = self.closeness;
        w.closeness_threshold = number(&self.threshold, "The closeness threshold")?;
        w.never_align = self.never_align;
        w.sort_rows = self.sort_rows;
        w.ignore_hidden_rows = self.ignore_hidden_rows;
        w.ignore_hidden_columns = self.ignore_hidden_columns;
        Ok(())
    }
}

/// A number field (`""`: none).
fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<Option<T>, String> {
    match word(text) {
        None => Ok(None),
        Some(t) => t
            .parse()
            .map(Some)
            .map_err(|_| format!("{what} must be a whole number, not `{t}`.")),
    }
}

/// The Rows tab.
#[derive(Debug, Clone, Default)]
pub struct RowsTab {
    /// The working copy.
    pub fields: RowsFields,
    original: RowsFields,
    /// Core's message, or a number that does not read.
    pub error: Option<String>,
}

impl SessionTab for RowsTab {
    fn title(&self) -> String {
        ROWS.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        let mut ok = true;
        let rules = with_table(current, |t| {
            ok = self.fields.write(&mut t.rules.rules).is_ok()
        });
        ok.then_some(rules)
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = words_of(rules) {
            self.fields = RowsFields::of(&t.rules.rules);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let mut result = Ok(());
        let rules = with_table(current, |t| result = self.fields.write(&mut t.rules.rules));
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

// ---- Parsing -----------------------------------------------------------------------------------------------------

/// The Parsing tab's fields (TB-001…TB-003): the parser words as typed, `""` for the sides' formats' own settings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsingFields {
    /// `detect`, `delimited`, `fixed`, `spreadsheet`.
    pub kind: String,
    /// Delimiters, one word a line (`,`, `;`, `tab`, `space`, `pipe`, …).
    pub delimiters: String,
    /// The text qualifier (`"`, `'`, `none`).
    pub qualifier: String,
    /// Consecutive delimiters are one.
    pub consecutive: bool,
    /// Fields are trimmed.
    pub trim: bool,
    /// Fixed widths (`10;20;8`).
    pub widths: String,
    /// `detect`, `names`, `data`.
    pub first_line: String,
    /// Decimal separator.
    pub decimal: String,
    /// Thousands separator (`none`).
    pub thousands: String,
    /// `ymd`, `dmy`, `mdy`.
    pub date_order: String,
    /// Date separator (`any`).
    pub date_sep: String,
    /// `system` or `default`.
    pub regional: String,
    /// The encoding label.
    pub encoding: String,
    /// A Ctrl+Z ends the file.
    pub ctrl_z: bool,
}

impl ParsingFields {
    fn of(w: &TableFormatWords) -> ParsingFields {
        let s = |o: &Option<String>| o.clone().unwrap_or_default();
        ParsingFields {
            kind: s(&w.kind),
            delimiters: joined(&w.delimiters),
            qualifier: s(&w.qualifier),
            consecutive: w.consecutive,
            trim: w.trim,
            widths: s(&w.widths),
            first_line: s(&w.first_line),
            decimal: s(&w.decimal),
            thousands: s(&w.thousands),
            date_order: s(&w.date_order),
            date_sep: s(&w.date_sep),
            regional: s(&w.regional),
            encoding: s(&w.encoding),
            ctrl_z: w.ctrl_z,
        }
    }

    fn write(&self, w: &mut TableFormatWords) {
        w.kind = word(&self.kind);
        w.delimiters = lines(&self.delimiters);
        w.qualifier = word(&self.qualifier);
        w.consecutive = self.consecutive;
        w.trim = self.trim;
        w.widths = word(&self.widths);
        w.first_line = word(&self.first_line);
        w.decimal = word(&self.decimal);
        w.thousands = word(&self.thousands);
        w.date_order = word(&self.date_order);
        w.date_sep = word(&self.date_sep);
        w.regional = word(&self.regional);
        w.encoding = word(&self.encoding);
        w.ctrl_z = self.ctrl_z;
    }
}

/// The Parsing tab.
#[derive(Debug, Clone, Default)]
pub struct ParsingTab {
    /// The working copy.
    pub fields: ParsingFields,
    original: ParsingFields,
    /// Core's message.
    pub error: Option<String>,
}

impl SessionTab for ParsingTab {
    fn title(&self) -> String {
        PARSING.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_table(current, |t| self.fields.write(&mut t.parse)))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = words_of(rules) {
            self.fields = ParsingFields::of(&t.parse);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let rules = with_table(current, |t| self.fields.write(&mut t.parse));
        self.error = core_check(svc, &rules);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_words() -> SheetRuleWords {
        SheetRuleWords {
            unimportant: vec!["note".into()],
            ignore_case: vec![String::new(), "name".into()],
            tolerance: vec!["0.5%".into(), "price=0.01".into()],
            date_tolerance: vec!["when=30s".into()],
            hide: vec!["note".into()],
            ..SheetRuleWords::default()
        }
    }

    /// The column rules round-trip through the tab's rows (grouped by column, the default row first).
    #[test]
    fn column_rules_round_trip() {
        let fields = ColumnsFields::of(&session_words());
        assert!(fields.default.ignore_case);
        assert_eq!(fields.default.tolerance, "0.5%");
        let names: Vec<&str> = fields.rows.iter().map(|r| r.columns.as_str()).collect();
        assert_eq!(names, ["note", "name", "price", "when"]);
        assert!(fields.rows[0].unimportant && fields.rows[0].hide);
        let mut back = SheetRuleWords::default();
        fields.write(&mut back);
        assert_eq!(back.unimportant, ["note"]);
        assert_eq!(back.hide, ["note"]);
        assert_eq!(back.ignore_case, ["", "name"]);
        assert_eq!(back.tolerance, ["0.5%", "price=0.01"]);
        assert_eq!(back.date_tolerance, ["when=30s"]);
        assert_eq!(ColumnsFields::of(&back), fields);
    }

    #[test]
    fn sheet_pairs_move_and_tidy() {
        let mut tab = SheetsTab::default();
        for p in ["a=b", " c=d ", "a=b", ""] {
            tab.new_pair = p.into();
            tab.add_pair();
        }
        assert_eq!(tab.fields.pairs, ["a=b", "c=d", "a=b"]);
        tab.selected = Some(1);
        tab.move_pair(-1);
        assert_eq!(tab.fields.pairs, ["c=d", "a=b", "a=b"]);
        tab.tidy();
        assert_eq!(tab.fields.pairs, ["c=d", "a=b"]);
    }

    #[test]
    fn rows_numbers_must_read() {
        let mut fields = RowsFields {
            skew: "lots".into(),
            ..RowsFields::default()
        };
        let mut w = SheetRuleWords::default();
        assert!(fields.write(&mut w).is_err());
        fields.skew = "40".into();
        fields.keys = "id, date".into();
        fields.write(&mut w).unwrap();
        assert_eq!(w.skew, Some(40));
        assert_eq!(w.keys, ["id, date"]);
    }

    #[test]
    fn only_table_sessions_have_these_tabs() {
        assert_eq!(rule_tabs(SessionType::TableCompare).len(), 4);
        assert!(rule_tabs(SessionType::TextCompare).is_empty());
    }
}
