//! File formats for Hikasa: the format model (FF-001…FF-040) with its TOML files, grammar compilation, the
//! tokenizer and its parallel speculative driver, line weights, the format registry (detection and
//! shadowing), the built-in presets, encoding detection and decoding, binary detection, and the conversions
//! None, Sorted and External.
//!
//! # Modules
//!
//! | Module | Content |
//! |---|---|
//! | [`model`] | [`FileFormat`] and its settings types (TOML, FF-004) |
//! | [`grammar`] | [`GrammarItem`], [`ItemKind`] and the [`RawGrammarItem`] TOML shim |
//! | [`toml_io`] | [`FORMAT_SCHEMA_VERSION`], [`parse_format`], [`to_toml`], [`unknown_keys`] |
//! | [`regex_engine`] | [`Pattern`], [`EngineKind`], [`RegexLimits`] (`regex` first, `fancy-regex` when needed) |
//! | [`compile`] | [`CompiledFormat`], [`CompiledGrammar`], [`ElementId`], [`ElementTable`] (D-41) |
//! | [`tokenize`] | [`Token`], [`LineState`], [`Tokenizer`], [`tokenize_all`] (§2.10) |
//! | [`registry`] | [`FormatRegistry`], detection, shadowing, `order.toml` (§2.12) |
//! | [`presets`] | [`PRESETS`] (embedded) and [`FormatRegistry::builtin`] |
//! | [`encoding`] | [`TextEncoding`], [`detect`], [`decode`], [`utf_pattern`] (§2.8, D-42) |
//! | [`binary`] | [`sniff`] and the magic-number table (§2.9) |
//! | [`convert`] | [`ProcessRunner`], [`StdProcessRunner`], [`build_argv`], [`run_external`] (§2.10.8, D-39) |
//! | [`error`] | [`FormatError`], [`PatternError`], [`ConvertError`] |
//!
//! Every engine here is a pure function of its arguments (T1). The one exception is the timeout polling of
//! [`StdProcessRunner`], the named allow-point for the monotonic clock; the private directory of an external
//! conversion is also named with a per-process counter, which never influences a result (§1.4.8). Parallel
//! work runs only on the `rayon::ThreadPool` passed in (T2, T7).
//!
//! ```
//! use hikasa_formats::{EnabledOverrides, FormatRegistry, Tokenizer, LineState};
//!
//! let registry = FormatRegistry::builtin().unwrap();
//! let detection = registry.detect("main.C", &EnabledOverrides::default()).unwrap();
//! assert_eq!(detection.format.name(), "C, C++, C#, ObjC Source");
//! let grammar = detection.format.grammar();
//! let mut tokens = Vec::new();
//! Tokenizer::new(grammar).tokenize_line("int x; // note", 0, LineState::Normal, &mut tokens);
//! let names: Vec<_> = tokens.iter().map(|t| grammar.elements().name(t.element)).collect();
//! assert_eq!(names, ["Keyword", "Everything Else", "Identifier", "Operator", "Everything Else", "Comment"]);
//! ```
//!
//! Specification: `docs/design/m1-contracts.md` §1.4 and the algorithms of §2.8–§2.12; user documentation in
//! `docs/10-file-formats.md`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod binary;
pub mod compile;
pub mod convert;
pub mod encoding;
pub mod error;
pub mod grammar;
pub mod model;
pub mod presets;
pub mod regex_engine;
pub mod registry;
pub mod tokenize;
pub mod toml_io;
mod weights;

pub use binary::{BinaryReason, BinaryVerdict, MAGIC, MagicKind, SNIFF_LEN, magic, sniff};
pub use compile::{CompiledFormat, CompiledGrammar, EVERYTHING_ELSE, ElementId, ElementTable};
pub use convert::{
    ConversionVars, ProcessRunner, StdProcessRunner, build_argv, run_external, sort_lines,
};
pub use encoding::{
    DecodeOptions, Decoded, DetectedEncoding, EncodingSource, TextEncoding, UTF_PATTERN_SAMPLE,
    bom_sniff, decode, detect, encode, utf_pattern,
};
pub use error::{ConvertError, FormatError, PatternError};
pub use grammar::{GrammarItem, GrammarItemError, ItemKind, RawGrammarItem};
pub use model::{
    Conversion, ElementInfo, ElementName, EncodingSetting, ExternalConversion, FileFormat,
    FilenameEncoding, FormatId, FormatKind, InvalidName, LineWeight, TableSettings, TextSettings,
};
pub use presets::{PRESETS, Preset};
pub use regex_engine::{EngineKind, Pattern, RegexLimits};
pub use registry::{
    DetectReason, Detection, EnabledOverrides, FormatOrder, FormatRegistry, ORDER_SCHEMA_VERSION,
    OrderItem, Origin, RegistryEntry, ShadowStatus,
};
pub use tokenize::{
    LineBytes, LineOutcome, LineSource, LineState, Token, TokenizedLines, Tokenizer, check_tokens,
    escape_token_text, tokenize_all, tokenize_all_with_chunk, write_token_dump,
};
pub use toml_io::{FORMAT_SCHEMA_VERSION, parse_format, to_toml, unknown_keys};

#[cfg(test)]
mod tests {
    use super::*;

    fn send_sync<T: Send + Sync>() {}

    /// Compiled formats and the registry are shared across threads (`Arc`, §1.4.4, §5.2).
    #[test]
    fn shared_types_are_send_and_sync() {
        send_sync::<CompiledFormat>();
        send_sync::<CompiledGrammar>();
        send_sync::<FormatRegistry>();
        send_sync::<Pattern>();
        send_sync::<FormatError>();
        send_sync::<ConvertError>();
        send_sync::<TokenizedLines>();
        send_sync::<StdProcessRunner>();
    }
}
