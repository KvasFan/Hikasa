//! Hex Compare for Hikasa (contract `docs/design/m2-contracts.md` §1.12, W12a): the lazy byte document, None,
//! Fast and Complete alignment, the verdict, rows and display filters, navigation and Go To, find and value
//! interpretation.
//!
//! | Module | Content |
//! |---|---|
//! | [`document`] | [`HexDocument`]: local files read in place, archive members through the temp cache, multi-GB files at once (D-326) |
//! | [`align`](mod@align) | [`HexAlignment`], segments, sections, statistics, thumbnail; [`align_none`], [`align_fast`], [`align_complete`] (D-327…D-330) |
//! | [`verdict`](mod@verdict) | [`verdict()`](fn@verdict), [`converted_verdict`]: `binary_same` / `binary_different` / conversion rows (D-331) |
//! | [`rows`] | [`HexView`], [`HexLayout`], [`HexRow`], [`HexCell`]: rows under a display filter (D-332) |
//! | [`nav`] | [`nav::step`], Go To ([`nav::parse_address`]), the Current Byte Address ([`nav::address_text`]) (D-333) |
//! | [`find`] | [`HexFinder`], [`HexPattern`], [`find::parse_hex_bytes`] (D-334) |
//! | [`values`] | [`values::at`], [`values::format`], [`values::format_with`] (D-335; W13a) |
//! | [`copy`] | [`copy::selection_text`]: Copy of a selection as hex pairs or characters (W13a, D-357) |
//! | [`vocab`] | the words of hex and their parsers |
//!
//! Every read goes through `hikasa-vfs`; the bulk passes (the None scan, Fast's chunking, the statistics pass, find)
//! read through its chunk visitor (D-325), so local files are mapped one chunk at a time inside one call and nothing
//! keeps a mapping. Every function is a deterministic function of its arguments (T1); parallel work never changes
//! a result (T2): the visitor's cuts are fixed and Complete's budget is charged in gap order.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod align;
pub mod copy;
pub mod document;
pub mod error;
pub mod find;
pub mod nav;
pub mod rows;
pub mod values;
pub mod verdict;
pub mod vocab;

pub use align::{
    AlignMode, HexAlignOptions, HexAlignment, HexStats, Section, SectionKind, Sections, Segment,
    SegmentKind, SegmentStore, ThumbBucket, Thumbnail, align, align_complete, align_fast,
    align_none, scan_none,
};
pub use document::{Converted, HexConversion, HexDocument, HexFileInfo, HexLoad, HexLoadOptions};
pub use error::{Changed, HexError, HexWarning, HexWarningCode};
pub use find::{Direction, HexFinder, HexPattern};
pub use rows::{ByteState, FilterCache, HexCell, HexLayout, HexRow, HexView, RowState};
pub use values::{Interpreted, ValueFormat, Values};
pub use verdict::{HexVerdict, converted_verdict, verdict};
pub use vocab::{CharSet, DisplayFilter, Radix, TextEncoding, VocabError};
