//! Text documents and Text Compare for Hikasa: the document model (TC-001…TC-003), the text rules
//! (TC-010…TC-022), the comparison engine (keys → intern → align → rows → sections → statistics), the
//! thumbnail and display filters (TC-032, TC-035, TC-040), a progressive observer for GUIs (ARCH-004), and the
//! rules-based content comparator that hikasa-folder receives by injection (FC-025).
//!
//! # Modules
//!
//! | Module | Content |
//! |---|---|
//! | [`document`] | [`TextDocument`] (rope + Hikasa's own CR/LF/CRLF block line table), [`load_document`], [`LoadOptions`] |
//! | [`live`] | [`LiveComparison`]: provisional splices and the exact incremental recompare (§1.10.10–§1.10.11) |
//! | [`edit`] | [`EditableDocument`] (edits, undo, marks, bookmarks), the editing [`edit::commands`], [`Editability`] |
//! | [`save`] | [`save::plan_save`], [`save::SaveConfirmation`], [`save::SaveSource`], [`save::execute_save`]: saving edited documents (§1.10.12–§1.10.14) |
//! | [`find`] | [`find::Finder`], [`find::replace_all`], [`find::find_in_comparison`], [`find::files::find_in_files`]: find, replace and Find in Files (§1.10.18–§1.10.19) |
//! | [`nav`], [`details`] | [`nav::step`] per browse mode, Go To, [`details::hex_details`], Alignment Details, the ruler (§1.10.20) |
//! | [`transform`](mod@transform) | [`Transform`], [`transform()`](fn@transform::transform): the conversions of TC-063 as one edit each (§1.10.15) |
//! | [`rules`] | [`TextRules`] (TOML), [`ImportanceRules`], [`AlignmentRules`], [`UnimportantItem`], [`TextRulesPatch`] |
//! | [`classify`] | [`CharClass`], [`ClassifiedSpan`]: importance classes and line keys (§2.13; D-13, D-14, D-53) |
//! | [`compare`](mod@compare) | [`compare()`](fn@compare::compare), [`TextComparison`] (rows, sections, inline, spans, navigation, thumbnail) |
//! | [`stats`] | [`TextStats`], [`ChangeCounts`] (D-18) |
//! | [`view`] | [`TextDisplayFilter`], [`ViewOptions`], [`FilteredView`] (§2.18) |
//! | [`quick`] | [`quick_compare_bytes`], [`RulesComparator`] (§2.19; D-12, D-15, D-25) |
//! | [`error`] | [`TextError`] |
//!
//! ```
//! use std::sync::Arc;
//! use hikasa_core::{ContentResult, Ctl};
//! use hikasa_formats::CompiledFormat;
//! use hikasa_text::{CompareInput, RowState, TextDocument, TextRules, compare};
//!
//! let left = Arc::new(TextDocument::from_text("alpha\nbeta\ngamma\n"));
//! let right = Arc::new(TextDocument::from_text("alpha\nBeta\ngamma\n"));
//! let input = CompareInput {
//!     left,
//!     right,
//!     format: Arc::new(CompiledFormat::everything_else()),
//!     rules: TextRules::default(),
//!     bytes_identical: false,
//!     force_tokenize: false,
//!     anchors: Vec::new(),
//!     right_format: None,
//!     ignored: Default::default(),
//! };
//! let cmp = compare(input, None, &Ctl::detached(), None).unwrap();
//! assert_eq!(cmp.row_count(), 3);
//! assert_eq!(cmp.row(hikasa_text::RowIdx(1)).unwrap().state, RowState::Changed);
//! assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
//! // The inline difference is the changed letter.
//! let inline = cmp.inline(hikasa_text::RowIdx(1)).unwrap();
//! assert_eq!(inline.left[0].range, 0..1);
//! ```
//!
//! Every engine is a deterministic function of its arguments (T1); parallel work runs only on the
//! `rayon::ThreadPool` passed in, and gives the same result as the sequential run (T2, T7). The only I/O entry
//! point is [`load_document`], which reads through a [`hikasa_vfs::Vfs`].
//!
//! Specification: `docs/design/m1-contracts.md` §1.5 and the algorithms of §2.13–§2.19; user documentation in
//! `docs/05-text-views.md`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod classify;
pub mod compare;
pub mod details;
pub mod document;
pub mod edit;
pub mod error;
pub mod find;
pub mod live;
pub mod nav;
pub mod quick;
pub mod rules;
pub mod save;
pub mod stats;
pub mod transform;
mod util;
pub mod view;

pub use classify::{CharClass, ClassifiedSpan};
pub use compare::{
    ClassifiedInline, CompareInput, CompareObserver, Direction, IgnoredLines, Importance,
    InlineRange, PartialRows, RangeSet, Row, RowIdx, RowState, SPAN_LIMIT_BYTES, Section,
    SectionId, SectionKind, TextComparison, ThumbCell, compare, compare_with_limits, ignored_check,
};
pub use document::{
    DocOrigin, EolStyle, EolSummary, LineEnding, LineIdx, LineMarks, LoadOptions, LoadStamp,
    MAX_DOCUMENT_BYTES, MAX_LINES, TextDocument, load_document,
};
pub use edit::{
    CopyEdit, EditBlock, EditCommand, EditError, EditOptions, EditOutcome, EditRequest,
    Editability, EditableDocument, GroupHint, LineDelta, LineMap, LineMark, SavedFile, Selection,
    SpliceSource, TextPos, UndoId,
};
pub use error::TextError;
pub use live::{
    Installed, LiveComparison, LiveKeys, PathCounters, RecompareJob, Recompared, compare_live,
};
pub use quick::{
    DEFAULT_RULES_MAX_BYTES, QuickOptions, RulesComparator, quick_compare_bytes,
    quick_compare_bytes_with,
};
pub use rules::{
    AlgorithmChoice, AlignmentRules, ImportanceRules, TEXT_RULES_SCHEMA_VERSION, TextPattern,
    TextRules, TextRulesPatch, UnimportantItem,
};
pub use stats::{ChangeCounts, TextStats};
pub use transform::{Transform, TransformOutcome, transform};
pub use view::{FilteredView, TextDisplayFilter, TextToggles, ViewItem, ViewOptions};

// Re-exports (§0.4 rule 8), so that hikasa-report and hikasa-cli need no extra manifest edges.
pub use hikasa_diff::{Algorithm, Anchor, AnchorSide, InlineDiff, InlineOptions, Run, RunKind};
pub use hikasa_formats::{
    BinaryReason, BinaryVerdict, CompiledFormat, DetectReason, DetectedEncoding, ElementId,
    ElementTable, EncodingSource, FormatId, FormatKind, LineState, MagicKind, RegexLimits,
    TextEncoding, Token,
};
