//! Table reports (TB-025, Table Compare contract §6.3): the summary, side-by-side and interleaved layouts as plain text
//! (with ANSI colours on a terminal) and as self-contained HTML, the XML dump, and the JSON document
//! `hikasa.table_compare.v1` with its `schema` key.
//!
//! Every renderer reads the [`TableComparison`] and the caller's [`TableSide`]s only (RPT-011) and writes to the
//! caller's writer; the output is a deterministic function of its inputs (INV-RPT-1): no clock, no locale, `\n` line
//! ends, widths from `unicode-width`.

mod interleaved;
mod sbs;
mod summary;
mod xml;

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::{BufWriter, Write};

use hikasa_core::{ContentKind, ContentResult, Pair, Side, Warning, WarningCode};
use hikasa_table::{
    CellState, ColumnPair, Importance, RowAlignmentUsed, RowFilter, RowKind, RowPair,
    SheetAlignment, SheetComparison, SheetSelector, SheetState, TableCompareOptions,
    TableComparison, TableWarning, TableWarningCode, Value,
};
use unicode_width::UnicodeWidthChar;

use crate::error::ReportError;
use crate::escape;
use crate::json::FormatJson;
use crate::meta::{Header, ReportMeta};
use crate::options::{HtmlStyle, OutputFormat};
use crate::summary_counts::SummaryCount;

/// Default title of table reports.
pub(crate) const TABLE_TITLE: &str = "Table Compare";

/// Layouts of a table report (TB-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableLayout {
    /// The statistics of every sheet pair and the totals.
    Summary,
    /// One table per sheet: the left cells, then the right cells of each row.
    SideBySide,
    /// One table per sheet: a changed pair's left row, then its right row.
    Interleaved,
    /// The `hikasa-report` XML dump (`type="table"`).
    Xml,
    /// The `hikasa.table_compare.v1` document with its `schema` key.
    Json,
}

impl TableLayout {
    /// Parses `summary`, `side-by-side` (`sbs`), `interleaved`, `xml`, `json`; ASCII case-insensitive, `_` equals
    /// `-`.
    pub fn parse(word: &str) -> Result<TableLayout, ReportError> {
        Ok(
            match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "summary" => TableLayout::Summary,
                "side-by-side" | "sbs" => TableLayout::SideBySide,
                "interleaved" => TableLayout::Interleaved,
                "xml" => TableLayout::Xml,
                "json" => TableLayout::Json,
                _ => {
                    return Err(ReportError::invalid(
                        "table report layout",
                        word,
                        TABLE_LAYOUTS,
                    ));
                }
            },
        )
    }

    /// `summary`, `side-by-side`, `interleaved`, `xml`, `json`.
    pub const fn name(self) -> &'static str {
        match self {
            TableLayout::Summary => "summary",
            TableLayout::SideBySide => "side-by-side",
            TableLayout::Interleaved => "interleaved",
            TableLayout::Xml => "xml",
            TableLayout::Json => "json",
        }
    }
}

/// The layout words of table reports (the `expected` of an unknown one).
pub(crate) const TABLE_LAYOUTS: &str = "summary, side-by-side, interleaved, xml, json";

/// Which cells the rows of the JSON document carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TableCells {
    /// A changed row's differing cells, an orphan row's non-empty cells, none of a same row.
    #[default]
    Differing,
    /// Every cell of every shown row.
    All,
}

impl TableCells {
    /// Parses `differing` or `all` (also with the prefix `cells-`); ASCII case-insensitive, `_` equals `-`.
    pub fn parse(word: &str) -> Result<TableCells, ReportError> {
        let w = word.trim().to_ascii_lowercase().replace('_', "-");
        match w.strip_prefix("cells-").unwrap_or(&w) {
            "differing" | "differences" => Ok(TableCells::Differing),
            "all" => Ok(TableCells::All),
            _ => Err(ReportError::invalid("cells", word, "differing, all")),
        }
    }

    /// `differing`, `all`.
    pub const fn as_str(self) -> &'static str {
        match self {
            TableCells::Differing => "differing",
            TableCells::All => "all",
        }
    }
}

/// Options of a table report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableReportOptions {
    /// Layout.
    pub layout: TableLayout,
    /// The rows shown (its `ignore_unimportant` is replaced by [`TableReportOptions::ignore_unimportant`]).
    pub display: RowFilter,
    /// Unimportant differences count as same (rows, cells, the verdict).
    pub ignore_unimportant: bool,
    /// Row numbers (the sheet's row labels) in side-by-side and interleaved. Default true.
    pub row_numbers: bool,
    /// Total width of plain-text side-by-side and interleaved (columns). Default 160.
    pub width: u16,
    /// Output format (HTML for summary, side-by-side and interleaved).
    pub format: OutputFormat,
    /// The cells of the JSON layout's rows.
    pub cells: TableCells,
}

impl Default for TableReportOptions {
    /// Side-by-side, every row, ignore-unimportant off, row numbers, width 160, plain text, the differing cells.
    fn default() -> TableReportOptions {
        TableReportOptions {
            layout: TableLayout::SideBySide,
            display: RowFilter::ALL,
            ignore_unimportant: false,
            row_numbers: true,
            width: 160,
            format: OutputFormat::Text { ansi: false },
            cells: TableCells::Differing,
        }
    }
}

/// What the caller knows of one side beyond the comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableSide {
    /// The canonical spec, shown in headers.
    pub spec: String,
    /// Size in bytes.
    pub size: Option<u64>,
    /// Modification time (JSON: RFC 3339 UTC).
    pub mtime: Option<jiff::Timestamp>,
    /// The Hikasa file format chosen for the side (`id`, `name`, `kind`, `reason`), if any.
    pub format: Option<FormatJson>,
    /// The side's table type was detected from its content (the format asked for *Detect*): the JSON `detected`.
    pub sniffed: bool,
}

/// The input of a table report or of the table JSON.
#[derive(Clone, Copy)]
pub struct TableSource<'a> {
    /// The comparison.
    pub comparison: &'a TableComparison,
    /// Its options.
    pub options: &'a TableCompareOptions,
    /// The two sides.
    pub sides: Pair<&'a TableSide>,
    /// Both inputs had identical bytes (the verdict `binary_same`).
    pub bytes_identical: bool,
}

impl TableSource<'_> {
    /// The verdict under `ignore_unimportant` (T-16).
    pub fn verdict(&self, ignore_unimportant: bool) -> ContentResult {
        verdict_of(
            self.comparison
                .kind(self.bytes_identical, ignore_unimportant),
        )
    }

    /// The side `side`.
    pub(crate) fn side(&self, side: Side) -> &TableSide {
        match side {
            Side::Left => self.sides.left,
            _ => self.sides.right,
        }
    }
}

/// The content result of a table verdict.
pub(crate) fn verdict_of(kind: ContentKind) -> ContentResult {
    match kind {
        ContentKind::BinarySame => ContentResult::BinarySame,
        ContentKind::RulesSame => ContentResult::RulesSame,
        ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
        _ => ContentResult::ImportantDifferences,
    }
}

/// The warning code of a table warning (contract §6.2: one to one); `None` for a code this build does not know.
pub fn table_warning_code(code: TableWarningCode) -> Option<WarningCode> {
    Some(match code {
        TableWarningCode::LossyDecode => WarningCode::LossyDecode,
        TableWarningCode::UnterminatedQualifier => WarningCode::UnterminatedQualifier,
        TableWarningCode::StrayAfterQualifier => WarningCode::StrayAfterQualifier,
        TableWarningCode::SheetSkipped => WarningCode::SheetSkipped,
        TableWarningCode::DuplicateKeys => WarningCode::DuplicateKeys,
        TableWarningCode::AlignmentDegraded => WarningCode::AlignmentDegraded,
        TableWarningCode::WorkBudgetExhausted => WarningCode::WorkBudgetExhausted,
        TableWarningCode::ClosenessBudgetExhausted => WarningCode::ClosenessBudgetExhausted,
        TableWarningCode::ClosenessIgnored => WarningCode::ClosenessIgnored,
        TableWarningCode::FormulaWithoutValue => WarningCode::FormulaWithoutValue,
        TableWarningCode::RuleNotApplied => WarningCode::RuleNotApplied,
        TableWarningCode::KeysNotApplied => WarningCode::KeysNotApplied,
        TableWarningCode::DelimiterMismatch => WarningCode::DelimiterMismatch,
        _ => return None,
    })
}

/// A table warning as a [`Warning`]: its message after `left, sheet "Data": ` (the side and the sheet when known);
/// `spec` is the side's spec when `sides` are given.
pub fn table_warning(w: &TableWarning, sides: Option<Pair<&TableSide>>) -> Option<Warning> {
    let code = table_warning_code(w.code)?;
    let mut message = String::new();
    if let Some(side) = w.side {
        message.push_str(side.as_str());
    }
    if let Some(sheet) = &w.sheet {
        if !message.is_empty() {
            message.push_str(", ");
        }
        let _ = write!(message, "sheet \"{sheet}\"");
    }
    if !message.is_empty() {
        message.push_str(": ");
    }
    message.push_str(&w.message);
    let spec = match (w.side, sides) {
        (Some(Side::Left), Some(s)) => Some(s.left.spec.clone()),
        (Some(Side::Right), Some(s)) => Some(s.right.spec.clone()),
        _ => None,
    };
    Some(Warning {
        code,
        message,
        spec,
    })
}

/// Renders a table report into `out`. Returns the warnings of the rendering itself (none today).
pub fn render_table_report(
    src: &TableSource<'_>,
    opts: &TableReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    let mut w = BufWriter::with_capacity(64 * 1024, out);
    match (opts.layout, opts.format) {
        (TableLayout::Summary, OutputFormat::Text { ansi }) => {
            summary::write_text(&mut w, src, opts, meta, ansi)?;
        }
        (TableLayout::Summary, OutputFormat::Html { mono }) => {
            summary::write_html(&mut w, src, opts, meta, mono, style)?;
        }
        (TableLayout::SideBySide, OutputFormat::Text { ansi }) => {
            sbs::write_text(&mut w, src, opts, meta, ansi)?;
        }
        (TableLayout::SideBySide, OutputFormat::Html { mono }) => {
            sbs::write_html(&mut w, src, opts, meta, mono, style)?;
        }
        (TableLayout::Interleaved, OutputFormat::Text { ansi }) => {
            interleaved::write_text(&mut w, src, opts, meta, ansi)?;
        }
        (TableLayout::Interleaved, OutputFormat::Html { mono }) => {
            interleaved::write_html(&mut w, src, opts, meta, mono, style)?;
        }
        (TableLayout::Xml, OutputFormat::Text { .. }) => xml::write(&mut w, src, opts, meta)?,
        (TableLayout::Json, OutputFormat::Text { .. }) => {
            let view = crate::json::TableViewOptions {
                filter: opts.display,
                cells: opts.cells,
                ignore_unimportant: opts.ignore_unimportant,
            };
            let doc = crate::json::TableReportJson::new(src, &view);
            serde_json::to_writer_pretty(&mut w, &doc).map_err(std::io::Error::other)?;
            w.write_all(b"\n")?;
        }
        (layout @ (TableLayout::Xml | TableLayout::Json), format @ OutputFormat::Html { .. }) => {
            return Err(ReportError::Unsupported {
                layout: layout.name(),
                format: format.name(),
            });
        }
    }
    w.flush()?;
    Ok(Vec::new())
}

// ---- shared pieces of the layouts -------------------------------------------------------------------------------

/// The filter of a report: its toggles with the report's ignore-unimportant.
pub(crate) fn filter_of(opts: &TableReportOptions) -> RowFilter {
    RowFilter {
        ignore_unimportant: opts.ignore_unimportant,
        ..opts.display
    }
}

/// The word of a filter: `all`, `mismatches`, `matches`, `none` (the report words, without `display-`), or `custom`
/// for another set of toggles.
pub(crate) fn display_word(filter: RowFilter) -> &'static str {
    let toggles = |f: RowFilter| (f.same, f.changed, f.left_only, f.right_only);
    let t = toggles(filter);
    if t == toggles(RowFilter::ALL) {
        "all"
    } else if t == toggles(RowFilter::DIFFERENCES) {
        "mismatches"
    } else if t == toggles(RowFilter::SAME) {
        "matches"
    } else if t == toggles(RowFilter::NONE) {
        "none"
    } else {
        "custom"
    }
}

/// The header wording of a filter.
pub(crate) fn display_words(filter: RowFilter) -> String {
    match display_word(filter) {
        "mismatches" => "differences".to_owned(),
        "matches" => "same".to_owned(),
        "custom" => {
            let mut on = Vec::new();
            for (set, name) in [
                (filter.same, "same"),
                (filter.changed, "changed"),
                (filter.left_only, "left-only"),
                (filter.right_only, "right-only"),
            ] {
                if set {
                    on.push(name);
                }
            }
            if on.is_empty() {
                "none".to_owned()
            } else {
                on.join(", ")
            }
        }
        word => word.to_owned(),
    }
}

/// `yes`, `no`.
const fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// The `Mode:` line of a table report.
pub(crate) fn table_mode(opts: &TableReportOptions) -> String {
    let display = match opts.layout {
        TableLayout::Summary => "all".to_owned(),
        _ => display_words(opts.display),
    };
    format!(
        "display: {display}; ignore unimportant: {}",
        yes_no(opts.ignore_unimportant)
    )
}

/// A sheet selector as the vocabulary writes it (`3`, `Data`, `name:3` for a sheet named with digits).
pub(crate) fn selector_text(s: &SheetSelector) -> String {
    match s {
        SheetSelector::Position(n) => n.to_string(),
        SheetSelector::Name(name)
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("name:{name}")
        }
        SheetSelector::Name(name) => name.clone(),
    }
}

/// The word of a sheet alignment: `by_name`, `left_name`, `right_name`, `position`, `custom`.
pub(crate) const fn sheets_word(a: &SheetAlignment) -> &'static str {
    match a {
        SheetAlignment::ByName => "by_name",
        SheetAlignment::LeftName => "left_name",
        SheetAlignment::RightName => "right_name",
        SheetAlignment::Position => "position",
        SheetAlignment::Custom(_) => "custom",
    }
}

/// The `Rules:` line of a table report: sheets, columns, keys, rows, the default column rule, formulas.
pub(crate) fn table_rules_summary(opts: &TableCompareOptions) -> String {
    let rules = &opts.default_sheet;
    let mut parts = Vec::new();
    let sheets = match &opts.sheets {
        SheetAlignment::Custom(pairs) => {
            let pairs: Vec<String> = pairs
                .iter()
                .map(|p| {
                    format!(
                        "{}={}",
                        p.left.as_ref().map(selector_text).unwrap_or_default(),
                        p.right.as_ref().map(selector_text).unwrap_or_default()
                    )
                })
                .collect();
            format!("custom ({})", pairs.join(", "))
        }
        other => sheets_word(other).replace('_', " "),
    };
    parts.push(format!("sheets: {sheets}"));
    let columns = match &rules.columns {
        hikasa_table::ColumnAlignment::Auto => "auto".to_owned(),
        hikasa_table::ColumnAlignment::Number => "by number".to_owned(),
        hikasa_table::ColumnAlignment::LeftName => "left names".to_owned(),
        hikasa_table::ColumnAlignment::RightName => "right names".to_owned(),
        hikasa_table::ColumnAlignment::Custom(pairs) => format!("custom ({} pairs)", pairs.len()),
    };
    parts.push(format!("columns: {columns}"));
    if rules.keys.is_empty() {
        let r = &rules.rows;
        let mut rows = r.algorithm.as_str().to_owned();
        if r.sort_rows {
            rows.push_str(", sorted");
        }
        if r.never_align {
            rows.push_str(", never align");
        }
        if let Some(c) = r.closeness {
            let _ = write!(rows, ", closeness {c}\u{2030}");
        }
        parts.push(format!("rows: {rows}"));
    } else {
        let keys: Vec<String> = rules.keys.iter().map(ToString::to_string).collect();
        parts.push(format!("keys: {}", keys.join(", ")));
    }
    let d = &rules.default_rule;
    if d.kind != hikasa_table::ColumnType::General {
        parts.push(format!("type: {}", d.kind.as_str()));
    }
    if d.unimportant {
        parts.push("every column unimportant".to_owned());
    }
    if d.ignore_case {
        parts.push("case unimportant".to_owned());
    }
    if d.ignore_whitespace {
        parts.push("whitespace unimportant".to_owned());
    }
    if let Some(t) = d.numeric_tolerance {
        parts.push(format!("tolerance {t}"));
    }
    if let Some(ms) = d.date_tolerance_ms {
        parts.push(format!("date tolerance {ms} ms"));
    }
    if d.exact_numbers {
        parts.push("exact numbers".to_owned());
    }
    if !rules.column_rules.is_empty() {
        let cols: Vec<String> = rules
            .column_rules
            .iter()
            .map(|(c, _)| c.to_string())
            .collect();
        parts.push(format!("column rules: {}", cols.join(", ")));
    }
    if !opts.sheet_rules.is_empty() {
        parts.push(format!("sheet rules: {}", opts.sheet_rules.len()));
    }
    if rules.ignore_hidden_rows {
        parts.push("hidden rows ignored".to_owned());
    }
    if rules.ignore_hidden_columns {
        parts.push("hidden columns ignored".to_owned());
    }
    parts.push(format!("formulas: {}", yes_no(opts.compare_formulas)));
    parts.join("; ")
}

impl ReportMeta {
    /// Header for a table report: `mode` from the options' display and ignore-unimportant, `filters` from the
    /// comparison options (sheets, columns, keys or rows, the default rule, formulas).
    pub fn for_table(
        src: &TableSource<'_>,
        opts: &TableReportOptions,
        generated_at: Option<jiff::Zoned>,
        generator: &str,
    ) -> ReportMeta {
        ReportMeta {
            title: None,
            generated_at,
            generator: generator.to_owned(),
            mode: Some(table_mode(opts)),
            filters: Some(table_rules_summary(src.options)),
            errors: Pair::new(None, None),
            folder_link: None,
            unsaved: hikasa_core::Pair::new(false, false),
        }
    }
}

/// `"<spec> (<source>, <n> sheets[, <encoding>])"`: the `Left:`/`Right:` value of the header.
pub(crate) fn side_description(src: &TableSource<'_>, side: Side) -> String {
    let wb = src.comparison.workbook(side);
    let n = wb.sheets().len();
    let mut out = format!(
        "{} ({}, {}",
        src.side(side).spec,
        wb.source().as_str(),
        crate::options::plural(n as u64, "sheet")
    );
    if let Some(enc) = wb.encoding() {
        out.push_str(", ");
        out.push_str(enc);
    }
    out.push(')');
    out
}

/// The comparison's warnings as [`Warning`]s.
pub(crate) fn warnings(src: &TableSource<'_>) -> Vec<Warning> {
    src.comparison
        .warnings()
        .iter()
        .filter_map(|w| table_warning(w, Some(src.sides)))
        .collect()
}

/// The RPT-004 header of a table report.
pub(crate) fn header(src: &TableSource<'_>, meta: &ReportMeta) -> Header {
    Header::new(
        meta,
        TABLE_TITLE,
        Pair::new(
            side_description(src, Side::Left),
            side_description(src, Side::Right),
        ),
        Vec::new(),
        &warnings(src),
    )
}

/// The verdict words of a table report.
pub(crate) fn verdict_words(result: &ContentResult) -> String {
    crate::text::summary_verdict_words(result)
}

/// The words of a sheet state.
pub(crate) const fn state_words(state: SheetState) -> &'static str {
    match state {
        SheetState::Same => "same",
        SheetState::Unimportant => "unimportant differences only",
        SheetState::Important => "important differences",
        SheetState::LeftOnly => "left only",
        SheetState::RightOnly => "right only",
    }
}

/// The word of a sheet state in the XML and the HTML classes (`left-only`, …).
pub(crate) const fn state_word(state: SheetState) -> &'static str {
    match state {
        SheetState::Same => "same",
        SheetState::Unimportant => "unimportant",
        SheetState::Important => "important",
        SheetState::LeftOnly => "left-only",
        SheetState::RightOnly => "right-only",
    }
}

/// The names of the key columns of an alignment.
pub(crate) fn key_names(sheet: &SheetComparison, keys: &[u32]) -> Vec<String> {
    keys.iter()
        .map(|k| {
            sheet
                .columns()
                .get(*k as usize)
                .map_or_else(|| (k + 1).to_string(), |c| c.name.clone())
        })
        .collect()
}

/// `keyed by id, date`, `standard alignment`, `standard alignment of sorted rows`, `one side only`.
pub(crate) fn alignment_words(sheet: &SheetComparison) -> String {
    match sheet.alignment() {
        RowAlignmentUsed::Keyed { keys } => {
            format!("keyed by {}", key_names(sheet, keys).join(", "))
        }
        RowAlignmentUsed::Engine { algorithm, sorted } => {
            let sorted = if *sorted { " of sorted rows" } else { "" };
            format!("{} alignment{sorted}", algorithm.as_str())
        }
        RowAlignmentUsed::Orphan => "one side only".to_owned(),
    }
}

/// The title of a sheet's section: `Data`, `Data → Daten` (renamed), with its 1-based indices.
pub(crate) fn sheet_title(src: &TableSource<'_>, pair: usize) -> String {
    let sheet = &src.comparison.sheets()[pair];
    let name = |side| {
        src.comparison
            .sheet(pair, side)
            .map(|s| s.name().to_owned())
    };
    match (name(Side::Left), name(Side::Right)) {
        (Some(l), Some(r)) if l != r => format!("{l} \u{2192} {r}"),
        (Some(l), _) => l,
        (None, Some(r)) => r,
        (None, None) => sheet.name().to_owned(),
    }
}

/// The marker of a row (as text reports mark lines): ` ` same, `|` important, `~` unimportant, `<`, `>` orphans.
pub(crate) fn marker(row: &RowPair, iu: bool) -> char {
    match row.kind() {
        RowKind::LeftOnly => '<',
        RowKind::RightOnly => '>',
        _ if !row.differs(iu) => ' ',
        _ => match row.importance() {
            Importance::Important => '|',
            _ => '~',
        },
    }
}

/// The state of a cell as the report shows it: unimportant is same under ignore-unimportant.
pub(crate) fn shown_state(state: CellState, iu: bool) -> CellState {
    if iu && state == CellState::Unimportant {
        CellState::Same
    } else {
        state
    }
}

/// The display text of a value, controls escaped.
pub(crate) fn cell_text(value: &Value<'_>) -> String {
    escape::escape_controls(&value.display()).into_owned()
}

/// The columns a grid shows: every comparison column that is not hidden.
pub(crate) fn shown_columns(sheet: &SheetComparison) -> Vec<(u32, &ColumnPair)> {
    sheet
        .columns()
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.hidden)
        .map(|(i, c)| (i as u32, c))
        .collect()
}

/// The letters of a column on a side (the sheet's own), `None` when the column is not on that side.
pub(crate) fn column_letter(
    src: &TableSource<'_>,
    pair: usize,
    column: &ColumnPair,
    side: Side,
) -> Option<String> {
    let c = match side {
        Side::Left => column.left,
        _ => column.right,
    }?;
    src.comparison.sheet(pair, side).map(|s| s.column_label(c))
}

/// The display of a sheet's visible rows: runs of shown rows and of hidden rows, in order.
pub(crate) enum Item {
    /// A shown row (an index of [`SheetComparison::rows`]).
    Row(u32),
    /// A run of hidden rows.
    Gap(u32),
}

/// The items of a sheet under `filter`.
pub(crate) fn items(sheet: &SheetComparison, filter: &RowFilter) -> Vec<Item> {
    let mut out = Vec::new();
    let mut gap = 0u32;
    for (i, row) in sheet.rows().iter().enumerate() {
        if filter.shows(row) {
            if gap > 0 {
                out.push(Item::Gap(gap));
                gap = 0;
            }
            out.push(Item::Row(i as u32));
        } else {
            gap += 1;
        }
    }
    if gap > 0 {
        out.push(Item::Gap(gap));
    }
    out
}

/// The "N rows hidden" text of a gap.
pub(crate) fn hidden_text(n: u32) -> String {
    if n == 1 {
        "\u{2026} 1 row hidden \u{2026}".to_owned()
    } else {
        format!("\u{2026} {n} rows hidden \u{2026}")
    }
}

/// Display width of a text without controls.
pub(crate) fn width(s: &str) -> usize {
    crate::cells::text_width(s)
}

/// `s` cut to `w` columns (an ellipsis marks a cut) and padded with spaces to exactly `w`.
pub(crate) fn fit(s: &str, w: usize) -> Cow<'_, str> {
    let have = width(s);
    if have == w {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(w + 4);
    if have < w {
        out.push_str(s);
    } else if w > 0 {
        let mut used = 0usize;
        for c in s.chars() {
            let cw = c.width().unwrap_or(0);
            if used + cw > w - 1 {
                break;
            }
            out.push(c);
            used += cw;
        }
        out.push('\u{2026}');
    }
    let now = width(&out);
    crate::cells::pad(&mut out, w.saturating_sub(now));
    Cow::Owned(out)
}

/// Column widths that share `available` columns (water filling): each column gets its natural width when the
/// narrower ones leave room, the rest share what is left equally; at least 1 each.
pub(crate) fn share_widths(natural: &[usize], available: usize) -> Vec<usize> {
    let n = natural.len();
    let mut out = vec![0usize; n];
    if n == 0 {
        return out;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (natural[i], i));
    let mut left = available;
    for (k, &i) in order.iter().enumerate() {
        let share = (left / (n - k)).max(1);
        let w = natural[i].clamp(1, share);
        out[i] = w;
        left = left.saturating_sub(w);
    }
    out
}

/// The summary counts of a table comparison (T11: the summaries and the XML read the same list).
pub(crate) fn table_counts(cmp: &TableComparison) -> Vec<SummaryCount> {
    let s = cmp.stats();
    let t = &s.totals;
    let c = |group, name, value| SummaryCount { group, name, value };
    vec![
        c("sheets", "paired", s.sheets_paired),
        c("sheets", "left-only", s.sheets_left_only),
        c("sheets", "right-only", s.sheets_right_only),
        c("sheets", "differing", s.sheets_differing),
        c("rows", "left", t.left_rows),
        c("rows", "right", t.right_rows),
        c("rows", "pairs", t.rows),
        c("rows", "same", t.same_rows),
        c("rows", "changed-important", t.changed_important),
        c("rows", "changed-unimportant", t.changed_unimportant),
        c("rows", "left-only-important", t.left_only_important),
        c("rows", "left-only-unimportant", t.left_only_unimportant),
        c("rows", "right-only-important", t.right_only_important),
        c("rows", "right-only-unimportant", t.right_only_unimportant),
        c("cells", "compared", t.cells_compared),
        c("cells", "same", t.cells_same),
        c("cells", "unimportant", t.cells_unimportant),
        c("cells", "important", t.cells_important),
        c("columns", "paired", t.columns_paired),
        c("columns", "left-only", t.columns_left_only),
        c("columns", "right-only", t.columns_right_only),
        c("header", "differences", t.header_differences),
        c("duplicate-keys", "left", t.duplicate_left),
        c("duplicate-keys", "right", t.duplicate_right),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        for (word, layout) in [
            ("summary", TableLayout::Summary),
            ("SIDE_BY_SIDE", TableLayout::SideBySide),
            ("sbs", TableLayout::SideBySide),
            ("interleaved", TableLayout::Interleaved),
            ("xml", TableLayout::Xml),
            ("json", TableLayout::Json),
        ] {
            assert_eq!(TableLayout::parse(word).unwrap(), layout);
        }
        assert!(TableLayout::parse("unified").is_err());
        assert_eq!(TableLayout::SideBySide.name(), "side-by-side");
        assert_eq!(TableCells::parse("cells-all").unwrap(), TableCells::All);
        assert_eq!(
            TableCells::parse("Differing").unwrap(),
            TableCells::Differing
        );
        assert!(TableCells::parse("some").is_err());
        let d = TableReportOptions::default();
        assert!(d.row_numbers && d.width == 160 && d.display == RowFilter::ALL);
    }

    #[test]
    fn widths_and_fitting() {
        assert_eq!(share_widths(&[3, 50, 4], 30), [3, 23, 4]);
        assert_eq!(share_widths(&[10, 10], 8), [4, 4]);
        assert_eq!(share_widths(&[0, 2], 10), [1, 2]);
        assert_eq!(fit("abcdef", 4), "abc\u{2026}");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("日本語", 4), "日\u{2026} ");
        assert_eq!(fit("x", 0), "");
        assert_eq!(hidden_text(1), "\u{2026} 1 row hidden \u{2026}");
        assert_eq!(selector_text(&SheetSelector::Name("12".into())), "name:12");
        assert_eq!(selector_text(&SheetSelector::Position(2)), "2");
    }

    #[test]
    fn filters_have_words() {
        assert_eq!(display_word(RowFilter::ALL), "all");
        assert_eq!(display_word(RowFilter::DIFFERENCES), "mismatches");
        assert_eq!(display_word(RowFilter::SAME), "matches");
        assert_eq!(display_word(RowFilter::NONE), "none");
        let custom = RowFilter {
            left_only: false,
            ..RowFilter::DIFFERENCES
        };
        assert_eq!(display_word(custom), "custom");
        assert_eq!(display_words(custom), "changed, right-only");
    }
}
