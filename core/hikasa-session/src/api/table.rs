//! Table Compare in the Service (Table Compare contract §6.4, T-30…): [`TableCompareRequest`] with the user's words
//! ([`TableFormatWords`], [`TableRuleWords`]), [`Service::table_compare`] and [`Service::table_compare_json`], the
//! mapping of a format's [`TableSettings`] onto the engine's [`TableFormat`], the A16 locale table
//! ([`regional_for_locale`]), the human output ([`TableCompareResponse::human_lines`]) and the runner of the
//! built-in *Spreadsheet text* conversion ([`SpreadsheetRunner`], T-22).
//!
//! Every vocabulary word of a table comparison is parsed here, by the parsers of `hikasa-table` (§5.7, check 7): the
//! adapters pass the words as typed.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use hikasa_core::{
    CancelToken, Cancelled, ContentComparator, ContentKind, ContentMethod, ContentOutcome,
    ContentResult, ContentSource, Ctl, ExitCode, Pair, Side, Spec, Warning, WarningCode,
};
use hikasa_formats::{
    BuiltinConversion, CompiledFormat, ConvertError, DateOrderSetting, Detection, EnabledOverrides,
    EncodingSetting, FirstLineSetting, FormatKind, FormatRegistry, ProcessRunner, RegionalSettings,
    TableSettings, TableType, TextEncoding,
};
use hikasa_report::json::{DetectReasonJson, FormatJson, TableCompareJson, TableViewOptions};
use hikasa_report::{TableCells, TableSide, TableSource, table_compare_json, table_warning};
use hikasa_table::{
    CellState, ColumnAlignment, ColumnRef, ColumnType, DateOrder, DelimitedOptions, FirstLine,
    FixedOptions, LoadOptions, LoadRequest, Regional, RowFilter, RowKind, RulePatch,
    SheetAlignment, SheetPair, SheetRules, SheetSelector, SheetState, TableComparator,
    TableCompareOptions, TableComparison, TableError, TableFormat, TableKind, TableKindWord,
    Tolerance, Value, compare_specs, load_bytes, parse_column_list, parse_column_pair,
    parse_column_value, parse_date_tolerance, parse_delimiters, parse_qualifier, parse_separator,
    parse_widths, spreadsheet_to_text,
};
use hikasa_vfs::EntryKind;
use serde::{Deserialize, Serialize};

use super::compare::load_failed;
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

// ---- the words -----------------------------------------------------------------------------------------------------

/// The parser flags of a table comparison as typed (TB-001…TB-003, §6.5): each one given replaces the setting of the
/// sides' formats, on both sides. A session keeps them in `[rules.parse]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableFormatWords {
    /// `--type detect|delimited|fixed|spreadsheet`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `--delimiter <d>` (repeatable; `,`, `tab`, `;,`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub delimiters: Vec<String>,
    /// `--qualifier <c>|none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// `--consecutive`.
    #[serde(skip_serializing_if = "is_false")]
    pub consecutive: bool,
    /// `--trim`.
    #[serde(skip_serializing_if = "is_false")]
    pub trim: bool,
    /// `--widths <w;w;…>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widths: Option<String>,
    /// `--first-line detect|names|data`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line: Option<String>,
    /// `--decimal <c>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decimal: Option<String>,
    /// `--thousands <c>|none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thousands: Option<String>,
    /// `--date-order ymd|dmy|mdy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_order: Option<String>,
    /// `--date-sep <c>|any`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_sep: Option<String>,
    /// `--regional system|default`: the host's regional settings (the A16 table) or the defaults, before the four
    /// flags above (T-31).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regional: Option<String>,
    /// `--encoding <label>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// `--ctrl-z`.
    #[serde(skip_serializing_if = "is_false")]
    pub ctrl_z: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The rule flags of one sheet pair as typed (TB-011…TB-013, §6.5). Each list holds the flag's values in order; an
/// empty value of `ignore_case`, `ignore_whitespace` and `exact_numbers` means every column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetRuleWords {
    /// `--columns auto|number|left-name|right-name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<String>,
    /// `--align-column <L>=<R>` (repeatable; makes the alignment custom).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub align_columns: Vec<String>,
    /// `--key <cols>` (repeatable; the lists join in order).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// `--unimportant <cols>`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unimportant: Vec<String>,
    /// `--text <cols>`: forced Text.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<String>,
    /// `--number <cols>`: type Number (T-24: leading zeros parse).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub number: Vec<String>,
    /// `--ignore-case [<cols>]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore_case: Vec<String>,
    /// `--ignore-whitespace [<cols>]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore_whitespace: Vec<String>,
    /// `--tolerance [<cols>=]<n>[%]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tolerance: Vec<String>,
    /// `--date-tolerance [<cols>=]<n>[ms|s|m|h|d]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub date_tolerance: Vec<String>,
    /// `--exact-numbers [<cols>]` (T-25).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exact_numbers: Vec<String>,
    /// `--hide <cols>` (display only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// `--algorithm unaligned|standard|myers|patience|histogram`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    /// `--skew <n>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skew: Option<u32>,
    /// `--closeness` (`Some(true)`), `--no-closeness` (`Some(false)`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closeness: Option<bool>,
    /// `--closeness-threshold <‰>` (turns closeness on).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closeness_threshold: Option<u16>,
    /// `--never-align`.
    #[serde(skip_serializing_if = "is_false")]
    pub never_align: bool,
    /// `--sort-rows`.
    #[serde(skip_serializing_if = "is_false")]
    pub sort_rows: bool,
    /// `--ignore-hidden-rows` (T-29).
    #[serde(skip_serializing_if = "is_false")]
    pub ignore_hidden_rows: bool,
    /// `--ignore-hidden-columns` (T-29).
    #[serde(skip_serializing_if = "is_false")]
    pub ignore_hidden_columns: bool,
}

/// The rules of the sheet pairs a selector names (sessions only: `[[rules.sheet]]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetRulesWords {
    /// The left (else the right) sheet: a 1-based position or a name (`name:` forces a name).
    pub sheet: String,
    /// The rules, on top of the comparison's.
    #[serde(flatten)]
    pub rules: SheetRuleWords,
}

/// The rule flags of a table comparison as typed: sheets, the rules of every pair, formulas, and (sessions) per-sheet
/// rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableRuleWords {
    /// `--sheets by-name|left-name|right-name|position`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheets: Option<String>,
    /// `--sheet <L>=<R>` (repeatable; exactly these pairs).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sheet_pairs: Vec<String>,
    /// `--formulas` (T-13).
    #[serde(skip_serializing_if = "is_false")]
    pub formulas: bool,
    /// The rules of every pair.
    #[serde(flatten)]
    pub rules: SheetRuleWords,
    /// Rules of particular pairs, on top of `rules` (first match wins).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sheet: Vec<SheetRulesWords>,
}

/// The rules of a `table-compare` session (`[rules]`): the parser words in `[rules.parse]`, the rule words beside them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableSessionRules {
    /// The parser words.
    #[serde(skip_serializing_if = "TableFormatWords::is_empty")]
    pub parse: TableFormatWords,
    /// The rule words.
    #[serde(flatten)]
    pub rules: TableRuleWords,
}

impl TableFormatWords {
    /// True when no word is given.
    pub fn is_empty(&self) -> bool {
        *self == TableFormatWords::default()
    }

    /// `other`'s words on top of these.
    pub fn overlay(&mut self, other: &TableFormatWords) {
        let set = |a: &mut Option<String>, b: &Option<String>| {
            if b.is_some() {
                a.clone_from(b);
            }
        };
        set(&mut self.kind, &other.kind);
        if !other.delimiters.is_empty() {
            self.delimiters.clone_from(&other.delimiters);
        }
        set(&mut self.qualifier, &other.qualifier);
        self.consecutive |= other.consecutive;
        self.trim |= other.trim;
        set(&mut self.widths, &other.widths);
        set(&mut self.first_line, &other.first_line);
        set(&mut self.decimal, &other.decimal);
        set(&mut self.thousands, &other.thousands);
        set(&mut self.date_order, &other.date_order);
        set(&mut self.date_sep, &other.date_sep);
        set(&mut self.regional, &other.regional);
        set(&mut self.encoding, &other.encoding);
        self.ctrl_z |= other.ctrl_z;
    }
}

// ---- the request and the response ------------------------------------------------------------------------------

/// `hikasa table compare` (§6.4).
#[derive(Debug, Clone, Default)]
pub struct TableCompareRequest {
    /// Left spec as typed.
    pub left: OsString,
    /// Right spec as typed.
    pub right: OsString,
    /// `--format <id|name>`: the format of both sides.
    pub format: Option<String>,
    /// `--left-format`, `--right-format` (D-281; `format` with either is `InvalidArgument`).
    pub formats: Pair<Option<String>>,
    /// The parser words.
    pub parse: TableFormatWords,
    /// The rule words.
    pub rules: TableRuleWords,
    /// `--ignore-unimportant` (verdict and exit code, T-16).
    pub ignore_unimportant: bool,
    /// `--display all|differences|same|none` (`None`: the caller's default).
    pub display: Option<String>,
    /// `--cells differing|all` (JSON).
    pub cells: Option<String>,
    /// FF-002: formats on or off for this comparison, id → enabled (a folder row's).
    pub format_overrides: std::collections::BTreeMap<String, bool>,
    /// `--verbose`: the informational warnings too (`merged_cells`).
    pub verbose: bool,
}

impl TableCompareRequest {
    /// A table comparison of two specs that were already parsed (a folder row).
    pub fn for_specs(left: &Spec, right: &Spec) -> TableCompareRequest {
        TableCompareRequest {
            left: super::spec_os_string(left),
            right: super::spec_os_string(right),
            ..TableCompareRequest::default()
        }
    }
}

/// The format a side was loaded with.
#[derive(Debug, Clone, PartialEq)]
pub struct TableFormatChoice {
    /// The Hikasa file format (`None`: no format of the Table kind matched; the engine's preset of the name).
    pub format: Option<FormatJson>,
    /// The engine's format asked for (before sniffing).
    pub table: TableFormat,
}

/// The result of [`Service::table_compare`].
#[derive(Debug, Clone)]
pub struct TableCompareResponse {
    /// The specs.
    pub specs: Pair<Spec>,
    /// What the reports and the JSON show of each side.
    pub sides: Pair<TableSide>,
    /// The formats each side was loaded with.
    pub formats: Pair<TableFormatChoice>,
    /// The comparison.
    pub comparison: Arc<TableComparison>,
    /// Its options.
    pub options: TableCompareOptions,
    /// Both inputs had identical bytes.
    pub bytes_identical: bool,
    /// `--ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// The verdict (T-16).
    pub kind: ContentKind,
    /// The exit code (1, 2, 12, 13).
    pub exit_code: ExitCode,
    /// The view of the JSON document and of the human output.
    pub view: TableViewOptions,
    /// Warnings (the engine's, as [`Warning`]s).
    pub warnings: Vec<Warning>,
}

impl TableCompareResponse {
    /// The input of the reports and of the JSON document.
    pub fn source(&self) -> TableSource<'_> {
        TableSource {
            comparison: &self.comparison,
            options: &self.options,
            sides: Pair::new(&self.sides.left, &self.sides.right),
            bytes_identical: self.bytes_identical,
        }
    }

    /// True when the view shows every row (`--display all`): the human output then lists them all.
    pub fn shows_all_rows(&self) -> bool {
        let f = self.view.filter;
        f.same && f.changed && f.left_only && f.right_only
    }

    /// The verdict as a content result.
    pub fn verdict(&self) -> ContentResult {
        self.source().verdict(self.ignore_unimportant)
    }

    /// `hikasa.table_compare.v1` (§6.3) with `view`.
    pub fn json(&self, view: &TableViewOptions) -> TableCompareJson {
        let mut view = *view;
        view.ignore_unimportant = self.ignore_unimportant;
        table_compare_json(&self.source(), &view)
    }

    /// The default human output (§6.5): a line per sheet pair (`= Sheet1`, `≠ Data: …`, `< OnlyLeft`), then the rows
    /// the view shows of each sheet as `row L/R: column: left → right` lines (at most `max_rows` per sheet, `None`: all),
    /// then the summary line.
    pub fn human_lines(&self, max_rows: Option<usize>) -> Vec<String> {
        let cmp = &*self.comparison;
        let iu = self.ignore_unimportant;
        let mut filter = self.view.filter;
        filter.ignore_unimportant = iu;
        let mut out = Vec::new();
        for (p, sheet) in cmp.sheets().iter().enumerate() {
            let stats = sheet.stats();
            let name = sheet_title(cmp, p);
            let state = sheet.state();
            let line = match state {
                SheetState::LeftOnly => format!("< {name} (left only)"),
                SheetState::RightOnly => format!("> {name} (right only)"),
                SheetState::Same => format!("= {name}"),
                SheetState::Unimportant if iu => format!("= {name} (unimportant differences only)"),
                SheetState::Unimportant | SheetState::Important => {
                    let mark = if state == SheetState::Important {
                        '\u{2260}'
                    } else {
                        '~'
                    };
                    let changed = stats.changed_important + stats.changed_unimportant;
                    let mut parts = Vec::new();
                    if changed > 0 {
                        parts.push(format!(
                            "{changed} {} differ",
                            plural(changed, "row", "rows")
                        ));
                    }
                    let left_only = stats.left_only_important + stats.left_only_unimportant;
                    if left_only > 0 {
                        parts.push(format!("{left_only} left only"));
                    }
                    let right_only = stats.right_only_important + stats.right_only_unimportant;
                    if right_only > 0 {
                        parts.push(format!("{right_only} right only"));
                    }
                    if stats.header_differences > 0 {
                        parts.push(format!(
                            "{} header {}",
                            stats.header_differences,
                            plural(stats.header_differences, "difference", "differences")
                        ));
                    }
                    if stats.columns_left_only + stats.columns_right_only > 0 {
                        parts.push(format!(
                            "columns: {} left only, {} right only",
                            stats.columns_left_only, stats.columns_right_only
                        ));
                    }
                    format!(
                        "{mark} {name}: {}; cells: {} important, {} unimportant",
                        if parts.is_empty() {
                            "differs".to_owned()
                        } else {
                            parts.join(", ")
                        },
                        stats.cells_important,
                        stats.cells_unimportant,
                    )
                }
            };
            out.push(line);
            if stats.duplicate_left + stats.duplicate_right > 0 {
                out.push(format!(
                    "  duplicate keys: {} left rows, {} right rows",
                    stats.duplicate_left, stats.duplicate_right
                ));
            }
            let columns = sheet.columns();
            let mut shown = 0usize;
            let mut more = 0usize;
            for (r, row) in sheet.rows().iter().enumerate() {
                if !filter.shows(row)
                    || matches!(state, SheetState::LeftOnly | SheetState::RightOnly)
                {
                    continue;
                }
                if max_rows.is_some_and(|max| shown >= max) {
                    more += 1;
                    continue;
                }
                shown += 1;
                let r = r as u32;
                let label = |side: Side| {
                    cmp.row_label(p, r, side)
                        .map_or_else(|| "-".to_owned(), |l| l.to_string())
                };
                let at = format!("  row {}/{}", label(Side::Left), label(Side::Right));
                match row.kind() {
                    RowKind::LeftOnly | RowKind::RightOnly => {
                        let side = if row.kind() == RowKind::LeftOnly {
                            Side::Left
                        } else {
                            Side::Right
                        };
                        let cells: Vec<String> = (0..columns.len() as u32)
                            .map(|c| show(&cmp.cell(p, r, c, side)))
                            .collect();
                        out.push(format!(
                            "{at}: {} only: {}",
                            side.as_str(),
                            cells.join(", ")
                        ));
                    }
                    RowKind::Same => out.push(format!("{at}: same")),
                    RowKind::Changed => {
                        for diff in sheet.differing_cells(r) {
                            if iu && diff.state == CellState::Unimportant {
                                continue;
                            }
                            let column = columns
                                .get(diff.column as usize)
                                .map_or_else(String::new, |c| c.name.clone());
                            let left = show(&cmp.cell(p, r, diff.column, Side::Left));
                            let right = show(&cmp.cell(p, r, diff.column, Side::Right));
                            let note = if diff.state == CellState::Unimportant {
                                " (unimportant)"
                            } else {
                                ""
                            };
                            out.push(format!("{at}: {column}: {left} \u{2192} {right}{note}"));
                        }
                    }
                }
            }
            if more > 0 {
                out.push(format!(
                    "  … {more} more {} (--display all shows every row)",
                    plural(more as u64, "row", "rows")
                ));
            }
        }
        out.push(self.summary_line());
        out
    }

    /// The summary line: the verdict, the sheets and the rows.
    pub fn summary_line(&self) -> String {
        let stats = self.comparison.stats();
        let t = &stats.totals;
        let verdict = match self.verdict() {
            ContentResult::BinarySame => "identical",
            ContentResult::RulesSame => "same",
            ContentResult::UnimportantOnly => "unimportant differences only",
            _ => "important differences",
        };
        let sheets = stats.sheets_paired + stats.sheets_left_only + stats.sheets_right_only;
        format!(
            "{verdict}: {} of {sheets} {} {}; rows: {} same, {} changed, {} left only, {} right only",
            stats.sheets_differing,
            plural(sheets, "sheet", "sheets"),
            if stats.sheets_differing == 1 {
                "differs"
            } else {
                "differ"
            },
            t.same_rows,
            t.changed_important + t.changed_unimportant,
            t.left_only_important + t.left_only_unimportant,
            t.right_only_important + t.right_only_unimportant,
        )
    }
}

fn plural(n: u64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// A cell as the human output shows it: its display text, `""` for an empty cell, control characters escaped.
fn show(value: &Value<'_>) -> String {
    match value {
        Value::Empty => "\"\"".to_owned(),
        v => hikasa_report::escape_controls(&v.display()).into_owned(),
    }
}

/// `Data`, or `Data → data` for a renamed pair.
fn sheet_title(cmp: &TableComparison, pair: usize) -> String {
    let sheet = &cmp.sheets()[pair];
    let left = cmp.sheet(pair, Side::Left).map(|s| s.name());
    let right = cmp.sheet(pair, Side::Right).map(|s| s.name());
    match (left, right) {
        (Some(l), Some(r)) if sheet.renamed() => format!("{l} \u{2192} {r}"),
        _ => sheet.name().to_owned(),
    }
}

// ---- the words, parsed ---------------------------------------------------------------------------------------------

fn invalid(message: String) -> ApiError {
    ApiError::InvalidArgument(message)
}

/// The column lists of a flag's values.
fn column_lists(flag: &str, values: &[String]) -> Result<Vec<ColumnRef>, ApiError> {
    let mut out = Vec::new();
    for value in values {
        out.extend(
            parse_column_list(value).map_err(|e| invalid(format!("{flag} {value:?}: {e}")))?,
        );
    }
    Ok(out)
}

/// A per-column flag whose empty value means every column: the default rule or a patch per column.
fn every_or_columns(
    flag: &str,
    values: &[String],
    rules: &mut SheetRules,
    set: impl Fn(&mut RulePatch),
) -> Result<(), ApiError> {
    for value in values {
        if value.trim().is_empty() {
            let mut patch = RulePatch::default();
            set(&mut patch);
            patch.apply(&mut rules.default_rule);
        } else {
            for column in column_lists(flag, std::slice::from_ref(value))? {
                let mut patch = RulePatch::default();
                set(&mut patch);
                rules.column_rules.push((column, patch));
            }
        }
    }
    Ok(())
}

/// `words` on top of `base` (§6.5: every flag replaces or adds to the base's rules).
pub fn sheet_rules_of(words: &SheetRuleWords, base: &SheetRules) -> Result<SheetRules, ApiError> {
    let mut rules = base.clone();
    if let Some(word) = &words.columns {
        rules.columns = ColumnAlignment::parse(word).map_err(invalid)?;
    }
    if !words.align_columns.is_empty() {
        let pairs = words
            .align_columns
            .iter()
            .map(|p| {
                parse_column_pair(p).map_err(|e| invalid(format!("--align-column {p:?}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        rules.columns = ColumnAlignment::Custom(pairs);
    }
    if !words.keys.is_empty() {
        rules.keys = column_lists("--key", &words.keys)?;
    }
    for (flag, values, kind) in [
        ("--text", &words.text, ColumnType::Text),
        ("--number", &words.number, ColumnType::Number),
    ] {
        for column in column_lists(flag, values)? {
            let patch = RulePatch {
                kind: Some(kind),
                ..RulePatch::default()
            };
            rules.column_rules.push((column, patch));
        }
    }
    for column in column_lists("--unimportant", &words.unimportant)? {
        let patch = RulePatch {
            unimportant: Some(true),
            ..RulePatch::default()
        };
        rules.column_rules.push((column, patch));
    }
    every_or_columns("--ignore-case", &words.ignore_case, &mut rules, |p| {
        p.ignore_case = Some(true);
    })?;
    every_or_columns(
        "--ignore-whitespace",
        &words.ignore_whitespace,
        &mut rules,
        |p| p.ignore_whitespace = Some(true),
    )?;
    every_or_columns("--exact-numbers", &words.exact_numbers, &mut rules, |p| {
        p.exact_numbers = Some(true);
    })?;
    for value in &words.tolerance {
        let (columns, amount) = parse_column_value(value)
            .map_err(|e| invalid(format!("--tolerance {value:?}: {e}")))?;
        let tolerance =
            Tolerance::parse(&amount).map_err(|e| invalid(format!("--tolerance: {e}")))?;
        let patch = RulePatch {
            numeric_tolerance: Some(Some(tolerance)),
            ..RulePatch::default()
        };
        apply_to(&mut rules, columns, patch);
    }
    for value in &words.date_tolerance {
        let (columns, amount) = parse_column_value(value)
            .map_err(|e| invalid(format!("--date-tolerance {value:?}: {e}")))?;
        let ms =
            parse_date_tolerance(&amount).map_err(|e| invalid(format!("--date-tolerance: {e}")))?;
        let patch = RulePatch {
            date_tolerance_ms: Some(Some(ms)),
            ..RulePatch::default()
        };
        apply_to(&mut rules, columns, patch);
    }
    if !words.hide.is_empty() {
        rules.hidden.extend(column_lists("--hide", &words.hide)?);
    }
    if let Some(word) = &words.algorithm {
        rules.rows.algorithm = hikasa_table::parse_algorithm(word).map_err(invalid)?;
    }
    if let Some(skew) = words.skew {
        rules.rows.skew = skew;
    }
    match (words.closeness, words.closeness_threshold) {
        (Some(false), Some(_)) => {
            return Err(invalid(
                "--closeness-threshold turns closeness on; it cannot be combined with --no-closeness".to_owned(),
            ));
        }
        (_, Some(threshold)) => {
            if threshold > 1000 {
                return Err(invalid(format!(
                    "--closeness-threshold {threshold}: a per-mille value is at most 1000"
                )));
            }
            rules.rows.closeness = Some(threshold);
        }
        (Some(false), None) => rules.rows.closeness = None,
        (Some(true), None) => {
            rules.rows.closeness.get_or_insert(500);
        }
        (None, None) => {}
    }
    rules.rows.never_align |= words.never_align;
    rules.rows.sort_rows |= words.sort_rows;
    rules.ignore_hidden_rows |= words.ignore_hidden_rows;
    rules.ignore_hidden_columns |= words.ignore_hidden_columns;
    Ok(rules)
}

/// A patch on the default rule (no columns) or on each column.
fn apply_to(rules: &mut SheetRules, columns: Option<Vec<ColumnRef>>, patch: RulePatch) {
    match columns {
        None => patch.apply(&mut rules.default_rule),
        Some(columns) => {
            for column in columns {
                rules.column_rules.push((column, patch));
            }
        }
    }
}

/// The comparison options of rule words (§6.5): sheets, the rules of every pair, per-sheet rules, formulas.
pub fn table_options_of(words: &TableRuleWords) -> Result<TableCompareOptions, ApiError> {
    let mut options = TableCompareOptions::default();
    if let Some(word) = &words.sheets {
        options.sheets = SheetAlignment::parse(word).map_err(invalid)?;
    }
    if !words.sheet_pairs.is_empty() {
        let pairs = words
            .sheet_pairs
            .iter()
            .map(|p| SheetPair::parse(p).map_err(|e| invalid(format!("--sheet {p:?}: {e}"))))
            .collect::<Result<Vec<_>, _>>()?;
        options.sheets = SheetAlignment::Custom(pairs);
    }
    options.compare_formulas = words.formulas;
    options.default_sheet = sheet_rules_of(&words.rules, &SheetRules::default())?;
    // T-30: the rules of every sheet skip a sheet whose columns they do not name (a typo that names no column of
    // any sheet is still refused); the rules of one sheet must name its columns.
    options.default_sheet.lenient_references = true;
    for per in &words.sheet {
        let selector = SheetSelector::parse(&per.sheet)
            .map_err(|e| invalid(format!("rules.sheet {:?}: {e}", per.sheet)))?;
        let mut rules = sheet_rules_of(&per.rules, &options.default_sheet)?;
        rules.lenient_references = false;
        options.sheet_rules.push((selector, rules));
    }
    Ok(options)
}

/// The display filter word (`all`, `differences`, `same`, `none`; `None`: `default`).
pub fn table_display_of(word: Option<&str>, default: RowFilter) -> Result<RowFilter, ApiError> {
    match word {
        Some(word) => RowFilter::parse(word).map_err(invalid),
        None => Ok(default),
    }
}

/// The parser words, parsed once (before anything is read).
#[derive(Debug, Clone, Default, PartialEq)]
struct ParsedFormatWords {
    kind: Option<TableKindWord>,
    delimiters: Option<Vec<char>>,
    qualifier: Option<Option<char>>,
    consecutive: bool,
    trim: bool,
    widths: Option<Vec<u32>>,
    first_line: Option<FirstLine>,
    decimal: Option<char>,
    thousands: Option<Option<char>>,
    date_order: Option<DateOrder>,
    date_sep: Option<Option<char>>,
    system_regional: Option<bool>,
    encoding: Option<String>,
    ctrl_z: bool,
}

fn parse_format_words(words: &TableFormatWords) -> Result<ParsedFormatWords, ApiError> {
    let mut delimiters: Option<Vec<char>> = None;
    for value in &words.delimiters {
        let more = parse_delimiters(value).map_err(|e| invalid(format!("--delimiter: {e}")))?;
        let all = delimiters.get_or_insert_with(Vec::new);
        for c in more {
            if !all.contains(&c) {
                all.push(c);
            }
        }
    }
    let regional = match words.regional.as_deref().map(str::trim) {
        None => None,
        Some(w) if w.eq_ignore_ascii_case("system") => Some(true),
        Some(w) if w.eq_ignore_ascii_case("default") => Some(false),
        Some(w) => {
            return Err(invalid(format!(
                "unknown regional settings `{w}` (expected system or default)"
            )));
        }
    };
    if let Some(label) = &words.encoding
        && TextEncoding::for_label(label).is_none()
    {
        return Err(ApiError::Format(
            hikasa_formats::FormatError::UnknownEncoding {
                label: label.clone(),
            },
        ));
    }
    let parsed = ParsedFormatWords {
        kind: words
            .kind
            .as_deref()
            .map(TableKindWord::parse)
            .transpose()
            .map_err(invalid)?,
        delimiters,
        qualifier: words
            .qualifier
            .as_deref()
            .map(parse_qualifier)
            .transpose()
            .map_err(|e| invalid(format!("--qualifier: {e}")))?,
        consecutive: words.consecutive,
        trim: words.trim,
        widths: words
            .widths
            .as_deref()
            .map(parse_widths)
            .transpose()
            .map_err(|e| invalid(format!("--widths: {e}")))?,
        first_line: words
            .first_line
            .as_deref()
            .map(FirstLine::parse)
            .transpose()
            .map_err(invalid)?,
        decimal: words
            .decimal
            .as_deref()
            .map(|w| parse_separator(w, None))
            .transpose()
            .map_err(|e| invalid(format!("--decimal: {e}")))?
            .flatten(),
        thousands: words
            .thousands
            .as_deref()
            .map(|w| parse_separator(w, Some("none")))
            .transpose()
            .map_err(|e| invalid(format!("--thousands: {e}")))?,
        date_order: words
            .date_order
            .as_deref()
            .map(DateOrder::parse)
            .transpose()
            .map_err(invalid)?,
        date_sep: words
            .date_sep
            .as_deref()
            .map(|w| parse_separator(w, Some("any")))
            .transpose()
            .map_err(|e| invalid(format!("--date-sep: {e}")))?,
        system_regional: regional,
        encoding: words.encoding.clone(),
        ctrl_z: words.ctrl_z,
    };
    if let (Some(d), Some(Some(t))) = (parsed.decimal, parsed.thousands)
        && d == t
    {
        return Err(invalid("--decimal and --thousands must differ".to_owned()));
    }
    match parsed.kind {
        Some(TableKindWord::Fixed) if parsed.delimiters.is_some() => {
            return Err(invalid(
                "--delimiter applies to delimited tables, not to --type fixed".to_owned(),
            ));
        }
        Some(TableKindWord::Delimited) if parsed.widths.is_some() => {
            return Err(invalid(
                "--widths applies to fixed-width tables, not to --type delimited".to_owned(),
            ));
        }
        _ if parsed.delimiters.is_some() && parsed.widths.is_some() => {
            return Err(invalid(
                "--delimiter (delimited) and --widths (fixed width) exclude each other".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(parsed)
}

// ---- formats -------------------------------------------------------------------------------------------------------

/// The regional settings of a locale value such as `de_DE.UTF-8` (A16; contract §6.4): the language and territory
/// pick a row of the table; an unknown locale, `C` and `POSIX` give the defaults (`.`, `,`, YMD, any separator).
pub fn regional_for_locale(locale: &str) -> Regional {
    let tag = locale
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .replace('-', "_");
    let (lang, territory) = tag.split_once('_').unwrap_or((tag.as_str(), ""));
    let lang = lang.to_ascii_lowercase();
    let territory = territory.to_ascii_uppercase();
    let r = |decimal: char, thousands: Option<char>, date_order: DateOrder, sep: char| Regional {
        decimal,
        thousands,
        date_order,
        date_sep: Some(sep),
    };
    use DateOrder::{Dmy, Mdy, Ymd};
    match (lang.as_str(), territory.as_str()) {
        ("en", "US" | "PH" | "") => r('.', Some(','), Mdy, '/'),
        ("en", "CA") => r('.', Some(','), Ymd, '-'),
        ("en", "ZA") => r(',', Some('\u{a0}'), Ymd, '/'),
        ("en", _) => r('.', Some(','), Dmy, '/'),
        ("de" | "it" | "fr" | "rm", "CH" | "LI") => r('.', Some('\''), Dmy, '.'),
        ("de", _) => r(',', Some('.'), Dmy, '.'),
        ("fr", "CA") => r(',', Some('\u{a0}'), Ymd, '-'),
        ("fr", _) => r(',', Some('\u{202f}'), Dmy, '/'),
        ("nl", _) => r(',', Some('.'), Dmy, '-'),
        ("it" | "es" | "pt" | "el" | "id" | "ro" | "ca" | "gl", _) => r(',', Some('.'), Dmy, '/'),
        (
            "da" | "nb" | "nn" | "no" | "fi" | "tr" | "ru" | "uk" | "pl" | "cs" | "sk" | "hr"
            | "sl" | "sr" | "bg" | "et" | "lv" | "lt" | "be" | "kk",
            _,
        ) => {
            let thousands = if matches!(lang.as_str(), "da" | "tr") {
                '.'
            } else {
                '\u{a0}'
            };
            r(',', Some(thousands), Dmy, '.')
        }
        ("hu", _) => r(',', Some('\u{a0}'), Ymd, '.'),
        ("sv", _) => r(',', Some('\u{a0}'), Ymd, '-'),
        ("ja" | "zh", _) => r('.', Some(','), Ymd, '/'),
        ("ko", _) => r('.', Some(','), Ymd, '.'),
        ("he" | "ar", _) => r('.', Some(','), Dmy, '/'),
        ("hi" | "bn" | "ta" | "te" | "mr", _) => r('.', Some(','), Dmy, '/'),
        _ => Regional::default(),
    }
}

/// The engine's regional settings of a format's.
fn regional_of(settings: &RegionalSettings, system: Regional) -> Regional {
    if settings.use_system {
        return system;
    }
    let one = |s: &str| s.chars().next();
    Regional {
        decimal: one(&settings.decimal).unwrap_or('.'),
        thousands: one(&settings.thousands),
        date_order: match settings.date_order {
            DateOrderSetting::Ymd => DateOrder::Ymd,
            DateOrderSetting::Dmy => DateOrder::Dmy,
            DateOrderSetting::Mdy => DateOrder::Mdy,
        },
        date_sep: one(&settings.date_sep),
    }
}

/// The engine's format of a Table format's settings (§6.1): the type, the delimiters (a `*.tsv` or `*.tab` file of a
/// format whose only delimiter is `,` takes tab, D-26's single entry), the widths, the first line, the regional
/// settings (`use_system`: the host's), the encoding (a fixed label; Auto lets the engine detect) and Ctrl+Z.
pub fn table_format_of(
    settings: &TableSettings,
    encoding: &EncodingSetting,
    name: &str,
    system: Regional,
) -> TableFormat {
    let lower = name.to_ascii_lowercase();
    let tab_file = lower.ends_with(".tsv") || lower.ends_with(".tab");
    let kind = match settings.kind {
        TableType::Detect => TableKind::Detect,
        TableType::Delimited => {
            let mut delimiters = settings.delimiter_set();
            if tab_file && delimiters == [','] {
                delimiters = vec!['\t'];
            }
            TableKind::Delimited(DelimitedOptions {
                delimiters,
                qualifier: settings.qualifier,
                consecutive_as_one: settings.consecutive_as_one,
                trim_whitespace: settings.trim_whitespace,
            })
        }
        TableType::Fixed => TableKind::Fixed(FixedOptions {
            widths: settings.widths.clone(),
        }),
        TableType::Spreadsheet => TableKind::Spreadsheet,
    };
    TableFormat {
        kind,
        first_line: match settings.first_line {
            FirstLineSetting::Detect => FirstLine::Detect,
            FirstLineSetting::Names => FirstLine::Names,
            FirstLineSetting::Data => FirstLine::Data,
        },
        regional: regional_of(&settings.regional, system),
        encoding: match encoding {
            EncodingSetting::Fixed { label } => Some(label.clone()),
            EncodingSetting::Auto { .. } => None,
        },
        ctrl_z_eof: settings.ctrl_z_eof,
    }
}

/// `words` on top of `format` (one side). A flag of delimited tables on a side whose type is detected is refused:
/// the sniffed options cannot take it (give `--type delimited` or `--delimiter`); a workbook side takes no parser flag
/// but the regional ones it does not use either, so they are ignored there.
fn apply_words(
    mut format: TableFormat,
    words: &ParsedFormatWords,
    system: Regional,
    side: Side,
) -> Result<TableFormat, ApiError> {
    match words.kind {
        Some(TableKindWord::Detect) => format.kind = TableKind::Detect,
        Some(TableKindWord::Spreadsheet) => format.kind = TableKind::Spreadsheet,
        Some(TableKindWord::Delimited) if !matches!(format.kind, TableKind::Delimited(_)) => {
            format.kind = TableKind::Delimited(DelimitedOptions::default());
        }
        Some(TableKindWord::Fixed) if !matches!(format.kind, TableKind::Fixed(_)) => {
            format.kind = TableKind::Fixed(FixedOptions::default());
        }
        _ => {}
    }
    if let Some(delimiters) = &words.delimiters {
        match &mut format.kind {
            TableKind::Delimited(opts) => opts.delimiters.clone_from(delimiters),
            TableKind::Spreadsheet if words.kind.is_none() => {}
            _ => {
                format.kind = TableKind::Delimited(DelimitedOptions {
                    delimiters: delimiters.clone(),
                    ..DelimitedOptions::default()
                });
            }
        }
    }
    if let Some(widths) = &words.widths {
        match &mut format.kind {
            TableKind::Spreadsheet if words.kind.is_none() => {}
            _ => {
                format.kind = TableKind::Fixed(FixedOptions {
                    widths: widths.clone(),
                })
            }
        }
    }
    let delimited_flags = words.qualifier.is_some() || words.consecutive || words.trim;
    match &mut format.kind {
        TableKind::Delimited(opts) => {
            if let Some(q) = words.qualifier {
                opts.qualifier = q;
            }
            opts.consecutive_as_one |= words.consecutive;
            opts.trim_whitespace |= words.trim;
            opts.validate()
                .map_err(|e| invalid(format!("{} side: {e}", side.as_str())))?;
        }
        TableKind::Detect if delimited_flags => {
            return Err(invalid(format!(
                "--qualifier, --consecutive and --trim apply to delimited tables, and the {} side's type is \
                 detected from its content: add --type delimited or --delimiter",
                side.as_str()
            )));
        }
        _ => {}
    }
    if let Some(first) = words.first_line {
        format.first_line = first;
    }
    match words.system_regional {
        Some(true) => format.regional = system,
        Some(false) => format.regional = Regional::default(),
        None => {}
    }
    if let Some(d) = words.decimal {
        format.regional.decimal = d;
    }
    if let Some(t) = words.thousands {
        format.regional.thousands = t;
    }
    if let Some(order) = words.date_order {
        format.regional.date_order = order;
    }
    if let Some(sep) = words.date_sep {
        format.regional.date_sep = sep;
    }
    if format.regional.thousands == Some(format.regional.decimal) {
        format.regional.thousands = None;
    }
    if let Some(label) = &words.encoding {
        format.encoding = Some(label.clone());
    }
    format.ctrl_z_eof |= words.ctrl_z;
    Ok(format)
}

/// The JSON of a detection.
fn format_json(detection: &Detection) -> FormatJson {
    FormatJson {
        id: detection.format.id().map(|id| id.as_str().to_owned()),
        name: detection.format.name().to_owned(),
        kind: detection.format.kind().as_str(),
        reason: Some(DetectReasonJson::new(&detection.reason)),
    }
}

/// The table settings of a compiled format of the Table kind (`None` for another kind).
fn table_settings(format: &CompiledFormat) -> Option<TableSettings> {
    if format.kind() != FormatKind::Table {
        return None;
    }
    Some(
        format
            .format()
            .and_then(|f| f.table.clone())
            .unwrap_or_default(),
    )
}

/// `--format` sets both sides: it cannot be combined with a side's own format (the Service's check and
/// `check_table_words`', phase-3 review F9).
fn format_combination(req: &TableCompareRequest) -> Result<(), ApiError> {
    if req.format.is_some() && (req.formats.left.is_some() || req.formats.right.is_some()) {
        return Err(invalid(
            "--format sets the format of both sides; it cannot be combined with --left-format or \
             --right-format"
                .to_owned(),
        ));
    }
    Ok(())
}

impl Service {
    /// The words of a table request checked as [`Service::table_compare`] checks them before anything is read (T-41):
    /// the rule and parser words, `--display`, `--cells` and the forced formats (known, of the Table kind). What the
    /// GUI's `OpenView` of `table compare --gui` checks locally (Table Compare phase 3, T-52). Additive.
    pub fn check_table_words(&self, req: &TableCompareRequest) -> Result<(), ApiError> {
        format_combination(req)?;
        table_options_of(&req.rules)?;
        parse_format_words(&req.parse)?;
        table_display_of(req.display.as_deref(), RowFilter::ALL)?;
        if let Some(word) = req.cells.as_deref() {
            TableCells::parse(word)?;
        }
        for name in [&req.format, &req.formats.left, &req.formats.right]
            .into_iter()
            .flatten()
        {
            let detection = self.registry().force(name)?;
            if detection.format.kind() != FormatKind::Table {
                return Err(invalid(format!(
                    "format `{}` is a {} format: table compare takes a format of the table kind",
                    detection.format.name(),
                    detection.format.kind()
                )));
            }
        }
        Ok(())
    }

    /// The host's regional settings (A16): `LC_ALL`, `LC_NUMERIC` for the numbers and `LC_TIME` for the dates as
    /// captured, else `LANG`, through [`regional_for_locale`].
    pub fn system_regional(&self) -> Regional {
        let env = self.env();
        let numbers = regional_for_locale(env.locale_numeric.as_deref().unwrap_or(""));
        let dates = regional_for_locale(env.locale_time.as_deref().unwrap_or(""));
        Regional {
            decimal: numbers.decimal,
            thousands: numbers.thousands,
            date_order: dates.date_order,
            date_sep: dates.date_sep,
        }
    }

    /// The format of each side (§6.4, T-30): `--left-format`/`--right-format`, else `--format`, else the side's own
    /// detection by name (a table pair is often an export against its source, `a.csv` against `b.xlsx`, so D-15's one
    /// format for the pair does not apply: the comparison never knows where a sheet came from, T-3). A forced format
    /// must be of the Table kind; a side whose name matches no Table format takes the engine's preset of the name.
    fn table_formats(
        &self,
        req: &TableCompareRequest,
        specs: Pair<&Spec>,
        words: &ParsedFormatWords,
    ) -> Result<Pair<TableFormatChoice>, ApiError> {
        format_combination(req)?;
        let registry = self.registry();
        let mut warnings = Vec::new();
        let overrides = self.enabled_overrides(&req.format_overrides, &mut warnings);
        let system = self.system_regional();
        let side = |side: Side,
                    spec: &Spec,
                    own: &Option<String>|
         -> Result<TableFormatChoice, ApiError> {
            let name = self.detection_name(spec).unwrap_or_default();
            let forced = own.as_deref().or(req.format.as_deref());
            let detection = match forced {
                Some(word) => {
                    let detection = registry.force(word)?;
                    if detection.format.kind() != FormatKind::Table {
                        return Err(invalid(format!(
                            "format `{}` is a {} format: table compare takes a format of the table kind",
                            detection.format.name(),
                            detection.format.kind()
                        )));
                    }
                    Some(detection)
                }
                None if name.is_empty() => None,
                None => registry.detect(&name, &overrides).ok(),
            };
            let base = match detection
                .as_ref()
                .and_then(|d| table_settings(&d.format).map(|t| (d, t)))
            {
                Some((d, settings)) => {
                    table_format_of(&settings, d.format.encoding(), &name, system)
                }
                None => TableFormat::from_name(&name),
            };
            let table = apply_words(base, words, system, side)?;
            Ok(TableFormatChoice {
                format: detection.as_ref().map(format_json),
                table,
            })
        };
        Ok(Pair::new(
            side(Side::Left, specs.left, &req.formats.left)?,
            side(Side::Right, specs.right, &req.formats.right)?,
        ))
    }

    /// Table compare (§6.4): the words parsed (a bad word is `InvalidArgument`, exit 100, before anything is read);
    /// the specs; each side's format (by its own name, T-31); both sides read through the router and loaded in
    /// parallel on the service pool, then compared (`hikasa_table::compare_specs`); the verdict and the exit code
    /// under `ignore_unimportant` (T-16: 1, 2, 12, 13). A column reference or a sheet that names nothing is
    /// `InvalidArgument`; an encrypted workbook `Unsupported`; any error exit 100.
    pub fn table_compare(
        &self,
        req: &TableCompareRequest,
        job: &JobControl,
    ) -> Result<TableCompareResponse, ApiError> {
        run_guarded(|| self.table_compare_inner(req, job))
    }

    fn table_compare_inner(
        &self,
        req: &TableCompareRequest,
        job: &JobControl,
    ) -> Result<TableCompareResponse, ApiError> {
        let options = table_options_of(&req.rules)?;
        let words = parse_format_words(&req.parse)?;
        let filter = table_display_of(req.display.as_deref(), RowFilter::DIFFERENCES)?;
        let cells = match req.cells.as_deref() {
            Some(word) => TableCells::parse(word)?,
            None => TableCells::Differing,
        };
        let left = self.parse_input(None, &req.left, &job.cancel)?;
        let right = self.parse_input(None, &req.right, &job.cancel)?;
        let mut entries = Vec::with_capacity(2);
        for (side, spec) in [(Side::Left, &left), (Side::Right, &right)] {
            let entry = self.stat_input(side, spec, job)?;
            if entry.kind == EntryKind::Dir {
                return Err(invalid(format!(
                    "{}: table compare compares files; use `folder compare` for folders",
                    spec.canonical()
                )));
            }
            entries.push(entry);
        }
        let formats = self.table_formats(req, Pair::new(&left, &right), &words)?;
        let load = LoadRequest {
            left: Some(formats.left.table.clone()),
            right: Some(formats.right.table.clone()),
            options: LoadOptions::default(),
        };
        let router = self.router();
        let ctl = job.ctl();
        let compared = self
            .pool()
            .install(|| compare_specs(&*router, &left, &right, &load, &options, &ctl));
        let compared = compared.map_err(|e| table_error(e, Pair::new(&left, &right)))?;
        if job.cancel.is_cancelled() {
            return Err(ApiError::Cancelled);
        }
        let comparison = Arc::new(compared.comparison);
        let side_of = |side: Side, spec: &Spec, entry: &hikasa_vfs::Entry| TableSide {
            spec: spec.canonical(),
            size: Some(entry.size),
            mtime: entry
                .mtime
                .and_then(|t| jiff::Timestamp::new(t.unix_seconds(), t.subsec_nanos() as i32).ok()),
            format: side_of_pair(&formats, side).format.clone(),
            sniffed: matches!(side_of_pair(&formats, side).table.kind, TableKind::Detect)
                && !matches!(
                    comparison.workbook(side).source(),
                    hikasa_table::SourceKind::Xlsx
                        | hikasa_table::SourceKind::Xlsm
                        | hikasa_table::SourceKind::Xlsb
                        | hikasa_table::SourceKind::Xls
                        | hikasa_table::SourceKind::Ods
                ),
        };
        let sides = Pair::new(
            side_of(Side::Left, &left, &entries[0]),
            side_of(Side::Right, &right, &entries[1]),
        );
        let kind = comparison.kind(compared.bytes_identical, req.ignore_unimportant);
        let exit_code = ExitCode::for_kind(kind, req.ignore_unimportant);
        let mut warnings: Vec<Warning> = comparison
            .warnings()
            .iter()
            .filter_map(|w| table_warning(w, Some(Pair::new(&sides.left, &sides.right))))
            .collect();
        if req.verbose {
            for side in [Side::Left, Side::Right] {
                for sheet in comparison.workbook(side).sheets() {
                    let merged = sheet.info().merged.len();
                    if merged > 0 {
                        warnings.push(Warning {
                            code: WarningCode::MergedCells,
                            message: format!(
                                "{}, sheet \"{}\": {merged} merged {} compared at their top-left cell",
                                side.as_str(),
                                sheet.name(),
                                if merged == 1 { "range" } else { "ranges" }
                            ),
                            spec: Some(side_of_pair(&sides, side).spec.clone()),
                        });
                    }
                }
            }
        }
        Ok(TableCompareResponse {
            specs: Pair::new(left, right),
            sides,
            formats,
            comparison,
            options,
            bytes_identical: compared.bytes_identical,
            ignore_unimportant: req.ignore_unimportant,
            kind,
            exit_code,
            view: TableViewOptions {
                filter,
                cells,
                ignore_unimportant: req.ignore_unimportant,
            },
            warnings,
        })
    }

    /// [`Service::table_compare`], then its `hikasa.table_compare.v1` document with the request's view (`--display`,
    /// default differences; `--cells`, default differing).
    pub fn table_compare_json(
        &self,
        req: &TableCompareRequest,
        job: &JobControl,
    ) -> Result<(TableCompareJson, TableCompareResponse), ApiError> {
        let resp = self.table_compare(req, job)?;
        Ok((resp.json(&resp.view), resp))
    }
}

/// The member of a pair on `side` (the right one for anything but the left).
fn side_of_pair<T>(pair: &Pair<T>, side: Side) -> &T {
    match side {
        Side::Left => &pair.left,
        _ => &pair.right,
    }
}

/// A table error as an API error: a read failure is `LoadFailed` with its side and spec; a cancellation `Cancelled`.
fn table_error(e: TableError, specs: Pair<&Spec>) -> ApiError {
    match e {
        TableError::Cancelled => ApiError::Cancelled,
        TableError::Load { side, source } => {
            if source.is_cancelled() {
                return ApiError::Cancelled;
            }
            let spec = if side == Side::Right {
                specs.right
            } else {
                specs.left
            };
            load_failed(side, spec, source)
        }
        TableError::InvalidInput(message) => ApiError::InvalidArgument(message),
        other => ApiError::Table(other),
    }
}

// ---- the folder content test (T-21) --------------------------------------------------------------------------------

/// The Rules content test of a folder comparison and of `quick --rules` (T-21): a pair whose format is of the Table
/// kind compares through `hikasa_table::TableComparator` (a table that does not load is that pair's `error`, never a
/// byte comparison), every other pair through the text rules comparator.
pub(crate) struct RulesDispatch {
    text: Arc<dyn ContentComparator>,
    table: TableComparator,
    registry: Arc<FormatRegistry>,
    overrides: EnabledOverrides,
}

impl RulesDispatch {
    /// The dispatcher of `text` (the text rules comparator) with the registry's Table formats.
    pub(crate) fn new(
        text: Arc<dyn ContentComparator>,
        registry: Arc<FormatRegistry>,
        overrides: EnabledOverrides,
        system: Regional,
    ) -> RulesDispatch {
        RulesDispatch {
            text,
            table: table_comparator(Arc::clone(&registry), overrides.clone(), system),
            registry,
            overrides,
        }
    }

    /// True when the pair's format (D-15: the left name, else the right one) is of the Table kind.
    fn is_table(&self, left: &str, right: &str) -> bool {
        let name = if left.is_empty() { right } else { left };
        self.registry
            .detect(name, &self.overrides)
            .is_ok_and(|d| d.format.kind() == FormatKind::Table)
    }
}

impl ContentComparator for RulesDispatch {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        self.text.size_limit()
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        if self.is_table(left.name(), right.name()) {
            self.table.compare(left, right, ctl)
        } else {
            self.text.compare(left, right, ctl)
        }
    }
}

/// The table comparator of the content test: default options, each file's format by its name (the Table formats of
/// the registry with `overrides`, else the engine's preset of the name).
pub(crate) fn table_comparator(
    registry: Arc<FormatRegistry>,
    overrides: EnabledOverrides,
    system: Regional,
) -> TableComparator {
    let mut comparator = TableComparator::new();
    comparator.formats = Arc::new(move |name: &str| {
        let detection = registry.detect(name, &overrides).ok()?;
        let settings = table_settings(&detection.format)?;
        Some(table_format_of(
            &settings,
            detection.format.encoding(),
            name,
            system,
        ))
    });
    comparator
}

// ---- the Spreadsheet text conversion -------------------------------------------------------------------------------

/// The Service's conversion runner (T-22, T-30): External programs through the adapter's runner; the built-in
/// *Spreadsheet text* converter through `hikasa_table` (every sheet of a workbook as TSV, a `--- <sheet> ---` line
/// between sheets, [`spreadsheet_to_text`]).
pub struct SpreadsheetRunner {
    inner: Arc<dyn ProcessRunner>,
}

impl SpreadsheetRunner {
    /// Wraps the adapter's runner.
    pub fn new(inner: Arc<dyn ProcessRunner>) -> SpreadsheetRunner {
        SpreadsheetRunner { inner }
    }
}

impl std::fmt::Debug for SpreadsheetRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpreadsheetRunner").finish_non_exhaustive()
    }
}

impl ProcessRunner for SpreadsheetRunner {
    fn run(
        &self,
        argv: &[OsString],
        timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        self.inner.run(argv, timeout, cancel)
    }

    fn convert_builtin(
        &self,
        conversion: BuiltinConversion,
        bytes: &[u8],
        name: &str,
        cancel: &CancelToken,
    ) -> Result<Vec<u8>, ConvertError> {
        match conversion {
            BuiltinConversion::Spreadsheet => {
                let format = TableFormat {
                    kind: TableKind::Spreadsheet,
                    first_line: FirstLine::Data,
                    ..TableFormat::default()
                };
                let workbook = load_bytes(
                    bytes.to_vec(),
                    name,
                    &format,
                    &LoadOptions::default(),
                    cancel,
                )
                .map_err(|e| match e {
                    TableError::Cancelled => ConvertError::Cancelled,
                    other => ConvertError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        other.to_string(),
                    )),
                })?;
                Ok(spreadsheet_to_text(&workbook).into_bytes())
            }
            other => self.inner.convert_builtin(other, bytes, name, cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_locale_table() {
        let de = regional_for_locale("de_DE.UTF-8");
        assert_eq!(
            (de.decimal, de.thousands, de.date_order, de.date_sep),
            (',', Some('.'), DateOrder::Dmy, Some('.'))
        );
        let fr = regional_for_locale("fr_FR.UTF-8@euro");
        assert_eq!(
            (fr.decimal, fr.thousands, fr.date_sep),
            (',', Some('\u{202f}'), Some('/'))
        );
        let us = regional_for_locale("en_US.UTF-8");
        assert_eq!(
            (us.decimal, us.date_order, us.date_sep),
            ('.', DateOrder::Mdy, Some('/'))
        );
        let gb = regional_for_locale("en_GB");
        assert_eq!((gb.decimal, gb.date_order), ('.', DateOrder::Dmy));
        assert_eq!(regional_for_locale("C"), Regional::default());
        assert_eq!(regional_for_locale(""), Regional::default());
        assert_eq!(regional_for_locale("xx_YY"), Regional::default());
        assert_eq!(regional_for_locale("sv-SE").date_order, DateOrder::Ymd);
    }

    #[test]
    fn rule_words_become_options() {
        let words = TableRuleWords {
            sheets: Some("position".into()),
            formulas: true,
            rules: SheetRuleWords {
                keys: vec!["id,2".into()],
                unimportant: vec!["note".into()],
                ignore_case: vec![String::new()],
                tolerance: vec!["price=0.5%".into(), "0.01".into()],
                date_tolerance: vec!["when=2h".into()],
                closeness: Some(false),
                algorithm: Some("myers".into()),
                ..SheetRuleWords::default()
            },
            sheet: vec![SheetRulesWords {
                sheet: "Data".into(),
                rules: SheetRuleWords {
                    keys: vec!["code".into()],
                    ..SheetRuleWords::default()
                },
            }],
            ..TableRuleWords::default()
        };
        let o = table_options_of(&words).unwrap();
        assert_eq!(o.sheets, SheetAlignment::Position);
        assert!(o.compare_formulas);
        let d = &o.default_sheet;
        assert_eq!(
            d.keys,
            [ColumnRef::Auto("id".into()), ColumnRef::Position(2)]
        );
        assert!(d.default_rule.ignore_case);
        assert_eq!(
            d.default_rule.numeric_tolerance,
            Some(Tolerance::Absolute(0.01))
        );
        assert_eq!(d.rows.closeness, None);
        assert_eq!(d.rows.algorithm, hikasa_diff::Algorithm::Myers);
        assert_eq!(d.column_rules.len(), 3);
        assert_eq!(o.sheet_rules.len(), 1);
        assert_eq!(o.sheet_rules[0].1.keys, [ColumnRef::Auto("code".into())]);
        assert_eq!(
            o.sheet_rules[0].1.rows.closeness, None,
            "per-sheet rules start from the default"
        );
        for bad in [
            TableRuleWords {
                sheets: Some("sideways".into()),
                ..TableRuleWords::default()
            },
            TableRuleWords {
                rules: SheetRuleWords {
                    tolerance: vec!["price=lots".into()],
                    ..SheetRuleWords::default()
                },
                ..TableRuleWords::default()
            },
            TableRuleWords {
                rules: SheetRuleWords {
                    closeness: Some(false),
                    closeness_threshold: Some(300),
                    ..SheetRuleWords::default()
                },
                ..TableRuleWords::default()
            },
            TableRuleWords {
                sheet_pairs: vec!["=".into()],
                ..TableRuleWords::default()
            },
        ] {
            let e = table_options_of(&bad).unwrap_err();
            assert_eq!(e.kind(), hikasa_core::ErrorKind::InvalidInput, "{e}");
        }
    }

    #[test]
    fn format_words_and_settings() {
        let settings = TableSettings::default();
        let auto = EncodingSetting::Auto { fallback: None };
        let csv = table_format_of(&settings, &auto, "a.CSV", Regional::default());
        assert!(matches!(&csv.kind, TableKind::Delimited(o) if o.delimiters == [',']));
        let tsv = table_format_of(&settings, &auto, "a.tsv", Regional::default());
        assert!(matches!(&tsv.kind, TableKind::Delimited(o) if o.delimiters == ['\t']));
        let words = parse_format_words(&TableFormatWords {
            delimiters: vec![";".into(), "tab".into()],
            qualifier: Some("none".into()),
            trim: true,
            decimal: Some(",".into()),
            thousands: Some("none".into()),
            date_order: Some("dmy".into()),
            ..TableFormatWords::default()
        })
        .unwrap();
        let f = apply_words(csv, &words, Regional::default(), Side::Left).unwrap();
        match &f.kind {
            TableKind::Delimited(o) => {
                assert_eq!(o.delimiters, [';', '\t']);
                assert_eq!(o.qualifier, None);
                assert!(o.trim_whitespace);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!((f.regional.decimal, f.regional.thousands), (',', None));
        let detect = TableFormat::from_name("x.txt");
        let trim = parse_format_words(&TableFormatWords {
            trim: true,
            ..TableFormatWords::default()
        })
        .unwrap();
        assert!(apply_words(detect.clone(), &trim, Regional::default(), Side::Right).is_err());
        let fixed = parse_format_words(&TableFormatWords {
            widths: Some("3;4".into()),
            ..TableFormatWords::default()
        })
        .unwrap();
        let f = apply_words(detect, &fixed, Regional::default(), Side::Left).unwrap();
        assert!(matches!(&f.kind, TableKind::Fixed(o) if o.widths == [3, 4]));
        let book = TableFormat::from_name("b.xlsx");
        let f = apply_words(book, &words, Regional::default(), Side::Left).unwrap();
        assert_eq!(
            f.kind,
            TableKind::Spreadsheet,
            "parser flags leave a workbook side alone"
        );
        for bad in [
            TableFormatWords {
                kind: Some("xml".into()),
                ..TableFormatWords::default()
            },
            TableFormatWords {
                delimiters: vec!["x y".into()],
                ..TableFormatWords::default()
            },
            TableFormatWords {
                widths: Some("1;0".into()),
                ..TableFormatWords::default()
            },
            TableFormatWords {
                decimal: Some(",".into()),
                thousands: Some(",".into()),
                ..TableFormatWords::default()
            },
            TableFormatWords {
                encoding: Some("no-such-encoding".into()),
                ..TableFormatWords::default()
            },
            TableFormatWords {
                delimiters: vec![",".into()],
                widths: Some("1".into()),
                ..TableFormatWords::default()
            },
        ] {
            assert!(parse_format_words(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn session_rules_round_trip_through_toml() {
        let rules = TableSessionRules {
            parse: TableFormatWords {
                delimiters: vec![";".into()],
                ..TableFormatWords::default()
            },
            rules: TableRuleWords {
                sheets: Some("position".into()),
                rules: SheetRuleWords {
                    keys: vec!["id".into()],
                    closeness: Some(false),
                    ..SheetRuleWords::default()
                },
                sheet: vec![SheetRulesWords {
                    sheet: "2".into(),
                    rules: SheetRuleWords {
                        tolerance: vec!["0.1".into()],
                        ..SheetRuleWords::default()
                    },
                }],
                ..TableRuleWords::default()
            },
        };
        let text = toml::to_string(&rules).unwrap();
        let back: TableSessionRules = toml::from_str(&text).unwrap();
        assert_eq!(back, rules, "{text}");
        assert_eq!(
            toml::to_string(&TableSessionRules::default()).unwrap(),
            "",
            "defaults write nothing"
        );
    }
}
