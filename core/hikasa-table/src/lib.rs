//! Table Compare for Hikasa (contract `docs/design/m2-table-contract.md`): delimited, fixed-width and spreadsheet
//! tables (xlsx, xlsm, xlsb, xls, ods through `calamine`), typed cells, sheet, column and row alignment, keyed
//! joins, tolerances, statistics and the result model the reports and the grid render.
//!
//! | Module | Content |
//! |---|---|
//! | [`format`] | [`TableFormat`], [`TableKind`], [`DelimitedOptions`], [`FixedOptions`], [`FirstLine`] (§2.1) |
//! | [`regional`] | [`Regional`], number and date parsing (T-19) |
//! | [`delimited`] | the in-house delimited and fixed parser, sniffing, the writer (§2) |
//! | [`workbook`] | the workbook loader through `calamine` (§3) |
//! | [`load`] | [`load_bytes`], [`load_workbook`] through the VFS (T-4) |
//! | [`compare`] | [`compare_workbooks`], [`compare_specs`], [`TableComparator`], the result model and navigation (§5) |
//! | [`sheet`] | [`Workbook`], [`Sheet`] and the packed [`Cell`]s (§4) |
//!
//! Every function is a deterministic function of its arguments (T1): no clock, environment or locale. Parallel
//! work uses the caller's rayon pool and never changes a result (T2).
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod cell;
pub mod compare;
pub mod delimited;
pub mod error;
pub mod format;
mod header;
pub mod load;
pub mod regional;
pub mod sheet;
pub mod workbook;

pub use cell::{Cell, CellError, CellKind, Value};
pub use compare::nav::{Dir, next_difference_cell, next_difference_row, next_difference_sheet};
pub use compare::options::{
    ColumnAlignment, ColumnRef, ColumnRule, ColumnType, RowAlignment, RulePatch, SheetAlignment,
    SheetPair, SheetRules, SheetSelector, TableCompareOptions, Tolerance, parse_algorithm,
    parse_column_list, parse_column_pair, parse_column_value, parse_date_tolerance,
};
pub use compare::result::{
    CellDiff, CellState, ColumnPair, DuplicateKeys, Importance, RowAlignmentUsed, RowFilter,
    RowKind, RowPair, SheetComparison, SheetState, SheetStats, TableComparison, TableStats,
};
pub use compare::{
    FormatResolver, LoadRequest, SpecComparison, TableComparator, compare_specs, compare_workbooks,
};
pub use delimited::{WriteLoss, load_text, write_delimited};
pub use error::{TableError, TableWarning, TableWarningCode};
pub use format::{
    DelimitedOptions, FirstLine, FixedOptions, TableFormat, TableKind, TableKindWord,
    parse_delimiters, parse_qualifier, parse_separator, parse_widths,
};
pub use load::{LoadOptions, load_bytes, load_workbook, read_spec, spec_name};
pub use regional::{DateOrder, Parsed, Regional};
pub use sheet::{
    ColumnKind, Region, RowView, Sheet, SheetBuilder, SheetInfo, SourceKind, Workbook,
    column_letters, parse_column_letters, spreadsheet_to_text,
};
pub use workbook::{SpreadsheetOptions, detect_workbook, load_spreadsheet};
