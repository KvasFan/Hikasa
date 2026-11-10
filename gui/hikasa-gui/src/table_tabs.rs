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

/// `""` → `None`, else the text exactly as it stands: nothing is trimmed, a space is a value (review F1, F2).
fn word(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

/// A word typed into an *Add* field: trimmed, `None` when blank.
fn typed(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// The entries of a list field, one a line, as written (a `\r` before the line break dropped); empty lines are no
/// entries.
fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A list's entries as the lines of its field.
fn joined(list: &[String]) -> String {
    list.join("\n")
}

/// A one-character value the eye cannot see, shown as the core's word for it (`space`, `tab`, …; review F2). The
/// word is written back only when the field is edited; an untouched field keeps the character (F1).
fn shown(value: &str) -> String {
    match value {
        " " => "space".to_owned(),
        "\t" => "tab".to_owned(),
        "\u{a0}" => "nbsp".to_owned(),
        "\u{202f}" => "nnbsp".to_owned(),
        other => other.to_owned(),
    }
}

/// `value()` into `target` when the field was edited (`now` differs from the session's reading `read`): every
/// untouched field keeps the session's own value exactly (review F1).
fn put<F: PartialEq, T>(now: &F, read: &F, target: &mut T, value: impl FnOnce() -> T) {
    if now != read {
        *target = value();
    }
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

impl SheetsFields {
    fn of(t: &TableSessionRules) -> SheetsFields {
        SheetsFields {
            mode: t.rules.sheets.clone().unwrap_or_default(),
            pairs: t.rules.sheet_pairs.clone(),
            formulas: t.rules.formulas,
        }
    }

    /// Writes the edited fields (F1).
    fn write(&self, t: &mut TableSessionRules) {
        let read = SheetsFields::of(t);
        put(&self.mode, &read.mode, &mut t.rules.sheets, || {
            word(&self.mode)
        });
        put(&self.pairs, &read.pairs, &mut t.rules.sheet_pairs, || {
            self.pairs.clone()
        });
        put(
            &self.formulas,
            &read.formulas,
            &mut t.rules.formulas,
            || self.formulas,
        );
    }
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
        if let Some(pair) = typed(&self.new_pair) {
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
        Some(with_table(current, |t| self.fields.write(t)))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(t) = words_of(rules) {
            self.fields = SheetsFields::of(t);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        self.error = core_check(svc, &with_table(current, |t| self.fields.write(t)));
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
    /// The columns (a reference or a comma-separated list, T-12) as written; `""` for the default row.
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

/// A flag of the column rules: its field of a row and its list of the words.
type Flag = (
    fn(&ColumnRuleRow) -> bool,
    fn(&mut ColumnRuleRow) -> &mut bool,
    fn(&mut SheetRuleWords) -> &mut Vec<String>,
);

/// A valued rule of the columns: its field of a row and its list of the words.
type Valued = (
    fn(&ColumnRuleRow) -> &str,
    fn(&mut ColumnRuleRow) -> &mut String,
    fn(&mut SheetRuleWords) -> &mut Vec<String>,
);

const FLAGS: [Flag; 7] = [
    (
        |r| r.unimportant,
        |r| &mut r.unimportant,
        |w| &mut w.unimportant,
    ),
    (|r| r.text, |r| &mut r.text, |w| &mut w.text),
    (|r| r.number, |r| &mut r.number, |w| &mut w.number),
    (
        |r| r.ignore_case,
        |r| &mut r.ignore_case,
        |w| &mut w.ignore_case,
    ),
    (
        |r| r.ignore_whitespace,
        |r| &mut r.ignore_whitespace,
        |w| &mut w.ignore_whitespace,
    ),
    (
        |r| r.exact_numbers,
        |r| &mut r.exact_numbers,
        |w| &mut w.exact_numbers,
    ),
    (|r| r.hide, |r| &mut r.hide, |w| &mut w.hide),
];

const VALUED: [Valued; 2] = [
    (|r| &r.tolerance, |r| &mut r.tolerance, |w| &mut w.tolerance),
    (
        |r| &r.date_tolerance,
        |r| &mut r.date_tolerance,
        |w| &mut w.date_tolerance,
    ),
];

/// The row key of a list entry's columns: `None` for every column (an empty or blank reference).
fn row_key(columns: Option<&str>) -> Option<&str> {
    columns.filter(|c| !c.trim().is_empty())
}

impl ColumnsFields {
    /// The row of `key` (`None`: the default row).
    fn row(&self, key: Option<&str>) -> Option<&ColumnRuleRow> {
        match key {
            None => Some(&self.default),
            Some(k) => self.rows.iter().find(|r| r.columns == k),
        }
    }

    /// The row of `key`, created on first use.
    fn row_mut(&mut self, key: Option<&str>) -> &mut ColumnRuleRow {
        let Some(k) = key else {
            return &mut self.default;
        };
        let at = match self.rows.iter().position(|r| r.columns == k) {
            Some(at) => at,
            None => {
                self.rows.push(ColumnRuleRow {
                    columns: k.to_owned(),
                    ..ColumnRuleRow::default()
                });
                self.rows.len() - 1
            }
        };
        &mut self.rows[at]
    }

    fn of(w: &SheetRuleWords) -> ColumnsFields {
        let mut out = ColumnsFields {
            mode: w.columns.clone().unwrap_or_default(),
            pairs: joined(&w.align_columns),
            ..ColumnsFields::default()
        };
        let mut w = w.clone();
        for (_, set, list) in FLAGS {
            for c in list(&mut w).iter() {
                *set(out.row_mut(row_key(Some(c)))) = true;
            }
        }
        // `[<cols>=]<value>` split as the core splits it; the last entry of a column wins.
        for (_, set, list) in VALUED {
            for entry in list(&mut w).iter() {
                let (cols, value) = hikasa_table::split_column_value(entry);
                *set(out.row_mut(row_key(cols))) = value.to_owned();
            }
        }
        out
    }

    /// Writes the fields back over the session's words `w`: an entry whose rule is unchanged stays as written and
    /// where it was (the `""` of every column included), an entry whose rule changed is rewritten in its place or
    /// dropped, a rule new to a row is appended. `write` of `of(w)` is `w` (review F1).
    fn write(&self, w: &mut SheetRuleWords) {
        let read = ColumnsFields::of(w);
        put(&self.mode, &read.mode, &mut w.columns, || word(&self.mode));
        put(&self.pairs, &read.pairs, &mut w.align_columns, || {
            lines(&self.pairs)
        });
        for (get, _, words) in FLAGS {
            let wanted = |key: Option<&str>| self.row(key).is_some_and(get);
            let list = words(w);
            list.retain(|c| wanted(row_key(Some(c))));
            if get(&self.default) && !list.iter().any(|c| row_key(Some(c)).is_none()) {
                list.push(String::new());
            }
            for r in self.rows.iter().filter(|r| get(r)) {
                if !list.iter().any(|c| c == &r.columns) {
                    list.push(r.columns.clone());
                }
            }
        }
        for (get, _, words) in VALUED {
            let list = words(w);
            let keys: Vec<Option<String>> = list
                .iter()
                .map(|e| row_key(hikasa_table::split_column_value(e).0).map(str::to_owned))
                .collect();
            let mut out = Vec::with_capacity(list.len());
            for (k, entry) in list.iter().enumerate() {
                let key = keys[k].as_deref();
                let value = self.row(key).map_or("", get);
                if value.is_empty() {
                    continue;
                }
                let last = !keys[k + 1..].iter().any(|later| later.as_deref() == key);
                if !last || hikasa_table::split_column_value(entry).1 == value {
                    out.push(entry.clone());
                    continue;
                }
                out.push(match hikasa_table::split_column_value(entry).0 {
                    Some(cols) => format!("{cols}={value}"),
                    None => value.to_owned(),
                });
            }
            if !get(&self.default).is_empty() && !keys.iter().any(Option::is_none) {
                out.push(get(&self.default).to_owned());
            }
            for r in self.rows.iter().filter(|r| !get(r).is_empty()) {
                if !keys
                    .iter()
                    .any(|k| k.as_deref() == Some(r.columns.as_str()))
                {
                    out.push(format!("{}={}", r.columns, get(r)));
                }
            }
            *list = out;
        }
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
        let Some(columns) = typed(&self.new_columns) else {
            return;
        };
        self.new_columns.clear();
        if let Some(at) = self.fields.rows.iter().position(|r| r.columns == columns) {
            self.selected = Some(at);
            return;
        }
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
    /// Key columns in priority order, one key a line as written (a line may itself be a comma-separated list).
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
            keys: joined(&w.keys),
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

    /// Writes the edited fields (F1); a number field that is not a number is the error.
    fn write(&self, w: &mut SheetRuleWords) -> Result<(), String> {
        let read = RowsFields::of(w);
        let skew = number(&self.skew, "The skew tolerance")?;
        let threshold = number(&self.threshold, "The closeness threshold")?;
        put(&self.keys, &read.keys, &mut w.keys, || lines(&self.keys));
        put(&self.algorithm, &read.algorithm, &mut w.algorithm, || {
            word(&self.algorithm)
        });
        put(&self.skew, &read.skew, &mut w.skew, || skew);
        put(&self.closeness, &read.closeness, &mut w.closeness, || {
            self.closeness
        });
        put(
            &self.threshold,
            &read.threshold,
            &mut w.closeness_threshold,
            || threshold,
        );
        put(
            &self.never_align,
            &read.never_align,
            &mut w.never_align,
            || self.never_align,
        );
        put(&self.sort_rows, &read.sort_rows, &mut w.sort_rows, || {
            self.sort_rows
        });
        put(
            &self.ignore_hidden_rows,
            &read.ignore_hidden_rows,
            &mut w.ignore_hidden_rows,
            || self.ignore_hidden_rows,
        );
        put(
            &self.ignore_hidden_columns,
            &read.ignore_hidden_columns,
            &mut w.ignore_hidden_columns,
            || self.ignore_hidden_columns,
        );
        Ok(())
    }
}

/// A number field (`""`: none).
fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<Option<T>, String> {
    match typed(text) {
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

/// The Parsing tab's fields (TB-001…TB-003): the parser words as written, `""` for the sides' formats' own settings;
/// a space or a tab shows as its word (F2).
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
    /// Thousands separator (`none`, `space`).
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
        let s = |o: &Option<String>| o.as_deref().map(shown).unwrap_or_default();
        ParsingFields {
            kind: s(&w.kind),
            delimiters: w
                .delimiters
                .iter()
                .map(|d| shown(d))
                .collect::<Vec<_>>()
                .join("\n"),
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

    /// Writes the edited fields (F1), as written (F2).
    fn write(&self, w: &mut TableFormatWords) {
        let read = ParsingFields::of(w);
        let text = |now: &String, read: &String, target: &mut Option<String>| {
            put(now, read, target, || word(now));
        };
        text(&self.kind, &read.kind, &mut w.kind);
        put(
            &self.delimiters,
            &read.delimiters,
            &mut w.delimiters,
            || lines(&self.delimiters),
        );
        text(&self.qualifier, &read.qualifier, &mut w.qualifier);
        put(
            &self.consecutive,
            &read.consecutive,
            &mut w.consecutive,
            || self.consecutive,
        );
        put(&self.trim, &read.trim, &mut w.trim, || self.trim);
        text(&self.widths, &read.widths, &mut w.widths);
        text(&self.first_line, &read.first_line, &mut w.first_line);
        text(&self.decimal, &read.decimal, &mut w.decimal);
        text(&self.thousands, &read.thousands, &mut w.thousands);
        text(&self.date_order, &read.date_order, &mut w.date_order);
        text(&self.date_sep, &read.date_sep, &mut w.date_sep);
        text(&self.regional, &read.regional, &mut w.regional);
        text(&self.encoding, &read.encoding, &mut w.encoding);
        put(&self.ctrl_z, &read.ctrl_z, &mut w.ctrl_z, || self.ctrl_z);
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
    use proptest::prelude::*;

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

    /// The column rules read into the tab's rows (grouped by column, the default row apart) and write back as they
    /// were.
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

    /// F1: the `""` of every column keeps its place, an edit rewrites its own entry in place and leaves the others.
    #[test]
    fn column_edits_keep_the_other_entries_in_place() {
        let w = SheetRuleWords {
            ignore_case: vec!["name".into(), String::new()],
            tolerance: vec!["price=0.01".into(), "0.5%".into(), "qty=1".into()],
            ..SheetRuleWords::default()
        };
        let mut fields = ColumnsFields::of(&w);
        let mut out = w.clone();
        fields.write(&mut out);
        assert_eq!(out, w);
        fields.default.tolerance = "1%".into();
        fields.rows[0].ignore_case = false;
        let mut out = w.clone();
        fields.write(&mut out);
        assert_eq!(out.ignore_case, [""]);
        assert_eq!(out.tolerance, ["price=0.01", "1%", "qty=1"]);
        fields.rows.retain(|r| r.columns != "qty");
        fields.default.tolerance.clear();
        let mut out = w.clone();
        fields.write(&mut out);
        assert_eq!(out.tolerance, ["price=0.01"]);
    }

    /// R15, R16 and every other field: an edited field reaches the words; an untouched one keeps them.
    #[test]
    fn every_edited_field_reaches_the_words() {
        let mut rows = RowsFields::of(&SheetRuleWords::default());
        rows.never_align = true;
        rows.sort_rows = true;
        rows.ignore_hidden_rows = true;
        rows.ignore_hidden_columns = true;
        rows.closeness = Some(false);
        rows.threshold = "700".into();
        rows.algorithm = "myers".into();
        let mut w = SheetRuleWords::default();
        rows.write(&mut w).unwrap();
        assert!(w.never_align && w.sort_rows && w.ignore_hidden_rows && w.ignore_hidden_columns);
        assert_eq!(
            (w.closeness, w.closeness_threshold, w.algorithm.as_deref()),
            (Some(false), Some(700), Some("myers"))
        );
        let mut parsing = ParsingFields::of(&TableFormatWords::default());
        parsing.ctrl_z = true;
        parsing.consecutive = true;
        parsing.trim = true;
        parsing.delimiters = "tab\n;".into();
        parsing.qualifier = "'".into();
        parsing.widths = "1;2".into();
        parsing.first_line = "names".into();
        parsing.date_order = "dmy".into();
        parsing.regional = "system".into();
        parsing.kind = "delimited".into();
        let mut p = TableFormatWords::default();
        parsing.write(&mut p);
        assert!(p.ctrl_z && p.consecutive && p.trim);
        assert_eq!(p.delimiters, ["tab", ";"]);
        assert_eq!(
            [
                &p.qualifier,
                &p.widths,
                &p.first_line,
                &p.date_order,
                &p.regional,
                &p.kind
            ]
            .map(|o| o.as_deref().unwrap_or_default()),
            ["'", "1;2", "names", "dmy", "system", "delimited"]
        );
        let mut t = TableSessionRules::default();
        let sheets = SheetsFields {
            mode: "position".into(),
            pairs: vec!["a=b".into()],
            formulas: true,
        };
        sheets.write(&mut t);
        assert_eq!(SheetsFields::of(&t), sheets);
    }

    /// F2: a space or a tab shows as its word and stays a space when the field is not edited; written edited, the
    /// word is what the core reads.
    #[test]
    fn whitespace_values_show_as_words_and_are_never_trimmed() {
        let w = TableFormatWords {
            thousands: Some(" ".into()),
            decimal: Some("\t".into()),
            delimiters: vec![" ".into(), ",".into()],
            ..TableFormatWords::default()
        };
        let mut fields = ParsingFields::of(&w);
        assert_eq!(fields.thousands, "space");
        assert_eq!(fields.decimal, "tab");
        assert_eq!(fields.delimiters, "space\n,");
        fields.encoding = "utf-8".into();
        let mut out = w.clone();
        fields.write(&mut out);
        assert_eq!(out.thousands.as_deref(), Some(" "));
        assert_eq!(out.decimal.as_deref(), Some("\t"));
        assert_eq!(out.delimiters, [" ", ","]);
        assert_eq!(out.encoding.as_deref(), Some("utf-8"));
        fields.date_sep = " ".into();
        fields.write(&mut out);
        assert_eq!(
            out.date_sep.as_deref(),
            Some(" "),
            "a typed space stays a space"
        );
    }

    /// F1: one key a line, as written.
    #[test]
    fn keys_are_one_a_line() {
        let w = SheetRuleWords {
            keys: vec!["id".into(), "name".into()],
            ..SheetRuleWords::default()
        };
        let mut fields = RowsFields::of(&w);
        assert_eq!(fields.keys, "id\nname");
        fields.skew = "40".into();
        let mut out = w.clone();
        fields.write(&mut out).unwrap();
        assert_eq!(out.keys, ["id", "name"]);
        assert_eq!(out.skew, Some(40));
        fields.keys = "name\r\n\nid, date".into();
        fields.write(&mut out).unwrap();
        assert_eq!(out.keys, ["name", "id, date"]);
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

    // ---- write(of(w)) == w (review F1) ----

    fn column() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            Just(" ".to_owned()),
            "[abc]{1,2}",
            "[ab] [ab]",
            "[ab],[ab]",
        ]
    }

    fn list(entry: impl Strategy<Value = String>) -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(entry, 0..5)
    }

    fn valued() -> impl Strategy<Value = String> {
        (column(), "[0-9.%]{1,3}", any::<bool>()).prop_map(|(cols, value, bare)| {
            if bare {
                value
            } else {
                format!("{cols}={value}")
            }
        })
    }

    fn opt_word() -> impl Strategy<Value = Option<String>> {
        prop::option::of(prop_oneof![
            "[a-z;,]{1,4}",
            Just(" ".to_owned()),
            Just("\t".to_owned())
        ])
    }

    prop_compose! {
        fn sheet_words()(
            flags in prop::array::uniform7(list(column())),
            values in prop::array::uniform2(list(valued())),
            columns in opt_word(),
            align in list("[a-c]=[a-c]"),
            keys in list("[a-c ,]{1,4}".prop_filter("one key a line", |k| !k.is_empty())),
            algorithm in opt_word(),
            skew in prop::option::of(0u32..500),
            closeness in prop::option::of(any::<bool>()),
            threshold in prop::option::of(0u16..1000),
            bools in prop::array::uniform4(any::<bool>()),
        ) -> SheetRuleWords {
            let [unimportant, text, number, ignore_case, ignore_whitespace, exact_numbers, hide] = flags;
            let [tolerance, date_tolerance] = values;
            SheetRuleWords {
                unimportant, text, number, ignore_case, ignore_whitespace, exact_numbers, hide,
                tolerance, date_tolerance, columns, align_columns: align, keys, algorithm, skew,
                closeness, closeness_threshold: threshold,
                never_align: bools[0], sort_rows: bools[1], ignore_hidden_rows: bools[2],
                ignore_hidden_columns: bools[3],
                ..SheetRuleWords::default()
            }
        }
    }

    prop_compose! {
        fn format_words()(
            words in prop::array::uniform11(opt_word()),
            delimiters in list(prop_oneof!["[,;|]", Just(" ".to_owned()), Just("\t".to_owned())]),
            bools in prop::array::uniform3(any::<bool>()),
        ) -> TableFormatWords {
            let [kind, qualifier, widths, first_line, decimal, thousands, date_order, date_sep, regional, encoding, _] =
                words;
            TableFormatWords {
                kind, qualifier, widths, first_line, decimal, thousands, date_order, date_sep, regional, encoding,
                delimiters, consecutive: bools[0], trim: bools[1], ctrl_z: bools[2],
                ..TableFormatWords::default()
            }
        }
    }

    /// The rows of the fields that hold a rule, sorted (the order the lists' entries give them is not a rule).
    fn rules_of(f: &ColumnsFields) -> (ColumnRuleRow, Vec<ColumnRuleRow>) {
        let none = |r: &ColumnRuleRow| {
            *r == ColumnRuleRow {
                columns: r.columns.clone(),
                ..ColumnRuleRow::default()
            }
        };
        let mut rows: Vec<ColumnRuleRow> = f.rows.iter().filter(|r| !none(r)).cloned().collect();
        rows.sort_by(|a, b| a.columns.cmp(&b.columns));
        (f.default.clone(), rows)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// `write(of(w)) == w` for every tab, and the fields written over nothing read back as the same rules.
        #[test]
        fn write_of_is_the_identity(w in sheet_words(), p in format_words(), sheets in opt_word(),
                                     pairs in list("[a-c]=[a-c]"), formulas in any::<bool>()) {
            let columns = ColumnsFields::of(&w);
            let mut out = w.clone();
            columns.write(&mut out);
            prop_assert_eq!(&out, &w);
            let mut fresh = SheetRuleWords::default();
            columns.write(&mut fresh);
            prop_assert_eq!(rules_of(&ColumnsFields::of(&fresh)), rules_of(&columns));

            let rows = RowsFields::of(&w);
            let mut out = w.clone();
            rows.write(&mut out).unwrap();
            prop_assert_eq!(&out, &w);
            let mut fresh = SheetRuleWords::default();
            rows.write(&mut fresh).unwrap();
            prop_assert_eq!(RowsFields::of(&fresh), rows);

            let parsing = ParsingFields::of(&p);
            let mut out = p.clone();
            parsing.write(&mut out);
            prop_assert_eq!(&out, &p);

            let mut t = TableSessionRules::default();
            t.rules.sheets = sheets;
            t.rules.sheet_pairs = pairs;
            t.rules.formulas = formulas;
            let fields = SheetsFields::of(&t);
            let mut out = t.clone();
            fields.write(&mut out);
            prop_assert_eq!(&out, &t);
        }
    }
}
