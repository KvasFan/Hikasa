//! Report generation for Hikasa (RPT-002, RPT-004, RPT-010, RPT-011): pure rendering from result models to bytes.
//!
//! - **Text reports** ([`render_text_report`]): the annotated unified diff (the CLI default), patches in normal,
//!   context and unified style (TP-003; exact: a row is context only when its two lines are byte-identical, so
//!   applied to the left file every patch reproduces the right one, whatever the comparison rules ignore),
//!   side-by-side plain text and self-contained HTML (RPT-010 colours, a legend, "N lines hidden" rows), the
//!   summary, and the statistics CSV (D-18).
//! - **Folder reports** ([`render_folder_report`]): the status list (the CLI default), side-by-side text and
//!   HTML with [`FolderColumns`], the summary (counts per state) and the `hikasa.folder_compare.v1` JSON document
//!   (D-29).
//! - **JSON DTOs** ([`json`], [`text_compare_json`], [`folder_compare_json`]): the stable `--json` result shapes of
//!   §4.3.2 and §4.3.3.
//! - **Header** ([`ReportMeta`]): title, the generation time injected by the caller (D-24), mode, rules and
//!   filters, specs and per-side errors (RPT-004).
//!
//! Every renderer reads the result models only (RPT-011) and writes only to the `&mut dyn Write` it is given. The
//! output is byte-identical for identical inputs on every platform (INV-RPT-1): `\n` line ends (CSV: `\r\n`), no
//! locale, no clock, column widths from `unicode-width`.
//!
//! ```
//! use std::sync::Arc;
//! use hikasa_core::Ctl;
//! use hikasa_report::{render_text_report, PatchStyle, ReportMeta, TextLayout, TextReportOptions};
//! use hikasa_text::{CompareInput, CompiledFormat, TextDocument, TextRules, compare};
//!
//! let input = CompareInput {
//!     left: Arc::new(TextDocument::from_text("a\nb\nc\n")),
//!     right: Arc::new(TextDocument::from_text("a\nB\nc\n")),
//!     format: Arc::new(CompiledFormat::everything_else()),
//!     rules: TextRules::default(),
//!     bytes_identical: false,
//!     force_tokenize: false,
//! };
//! let cmp = compare(input, None, &Ctl::detached(), None)?;
//! let opts = TextReportOptions {
//!     layout: TextLayout::Patch(PatchStyle::Unified { lines: 1 }),
//!     ..TextReportOptions::default()
//! };
//! let meta = ReportMeta::for_text(&cmp, &opts, None, "hikasa doc");
//! let mut out = Vec::new();
//! render_text_report(&cmp, &opts, &meta, &mut out)?;
//! let text = String::from_utf8(out)?;
//! assert!(text.ends_with("--- <memory>\n+++ <memory>\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Additions to the M1 contract
//!
//! Additive public API beyond §1.7 (no signature of §1.7 changes): [`ReportError::InvalidOption`] and
//! `Classify for ReportError`; the vocabulary parsers [`DisplayMode::parse`], [`Wrap::parse`],
//! [`OutputFormat::parse`], [`PatchStyle::parse`], [`TextLayout::parse`], [`FolderLayout::parse`] and
//! [`FolderColumns::parse_list`] (SCR-001; T11) with their names ([`DisplayMode::describe`], [`Wrap::as_str`],
//! [`OutputFormat::name`], [`PatchStyle::as_str`], [`TextLayout::name`], [`FolderLayout::name`],
//! [`FolderColumns::names`]); `Default` for [`DisplayMode`], [`OutputFormat`], [`TextReportOptions`],
//! [`FolderReportOptions`] and [`FolderColumns`] (SIZE | MODIFIED); [`HtmlStyle`] (custom CSS, RPT-002
//! `html-custom`, API only in M1) with [`render_text_report_with`] (which also returns the rendering's
//! `patch_lossy_eol` and `patch_encoding` warnings) and [`render_folder_report_with`]; the DTO building blocks of
//! [`json`] ([`json::FormatJson`], [`json::DetectReasonJson`], the schema ids,
//! [`json::TextCompareJson::with_detection`] for the detection reasons only the caller knows, and
//! [`json::FolderReportJson`], the JSON folder report with its `schema` key); wave 4: [`ReportMeta::with_title`]
//! (`--title`), [`parse_folder_mode`] and [`folder_mode_word`] (`--folders always|structure|files-only`). The
//! complete list, with the accepted interpretations, is §1.7.3 of the contract.
//!
//! Specification: `docs/design/m1-contracts.md` §1.7, §3.4 and §4.3; user documentation: `docs/13-reports.md`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod cells;
mod error;
mod escape;
pub mod folder;
pub mod json;
mod meta;
mod options;
mod style;
pub mod text;

pub use error::ReportError;
pub use folder::{
    FolderColumns, FolderLayout, FolderReportOptions, render_folder_report,
    render_folder_report_with,
};
pub use json::{folder_compare_json, text_compare_json};
pub use meta::ReportMeta;
pub use options::{
    DisplayMode, HtmlStyle, OutputFormat, Wrap, folder_mode_word, parse_folder_mode,
};
pub use text::{
    PatchStyle, TextLayout, TextReportOptions, render_text_report, render_text_report_with,
};
