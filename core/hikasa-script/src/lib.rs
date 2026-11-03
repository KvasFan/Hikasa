//! The script language (§1.8, D-215): lexer, parser, checked AST, `%`-expansion, `lt`/`rt`, the flow check and the
//! interpreter loop over a [`ScriptHost`]. Pure: no file, process, network, clock or environment; every vocabulary word is
//! judged through a [`WordChecker`] and every command runs through the host, both implemented by the session layer with
//! the parsers and calls the CLI uses (T11).
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod ast;
pub mod check;
pub mod error;
pub mod expand;
pub mod interp;
pub mod lex;
pub mod parse;

use serde::Serialize;

pub use ast::{
    Args, Command, CommandName, CopyToArgs, ExpandArgs, FilterArgs, LoadArgs, LogArgs, OptionArgs,
    RenameArgs, ReportArgs, ReportFor, Script, Slot, SyncArgs, SyncMode, TouchArgs,
};
pub use check::{AnyWord, RefCheck, WordChecker, check_word, parse, parse_checked, script_syntax};
pub use error::{Pos, ScriptError, ScriptErrorCode};
pub use expand::{ExpandContext, LocalTime, References, expand, has_forms, normalize_sides};
pub use interp::{
    CommandOutcome, CommandRecord, CommandStatus, ConfirmWord, HostUi, LineKind, LogLevel,
    Resolved, RunOptions, RunOutcome, RunState, ScriptEvent, ScriptHost, StopAt, StopReason,
    display_os, elapsed_text, run,
};
pub use lex::{Line, Word, lex, split_words};

/// The largest script read: 16 MiB (§1.8.3); a larger one is 105 `too_large`.
pub const SCRIPT_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The most errors a check reports; the rest are counted (§1.8.7).
pub const SCRIPT_MAX_ERRORS: usize = 100;

/// The errors of a check: the first [`SCRIPT_MAX_ERRORS`] in line order, and how many more there were.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Errors {
    /// The errors kept.
    pub list: Vec<ScriptError>,
    /// The errors past the cap.
    pub more: u64,
}

impl Errors {
    /// Adds an error: kept under the cap, else counted.
    pub fn push(&mut self, error: ScriptError) {
        if self.list.len() < SCRIPT_MAX_ERRORS {
            self.list.push(error);
        } else {
            self.more += 1;
        }
    }

    /// No error.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.more == 0
    }

    /// Every error, those past the cap included.
    pub fn total(&self) -> u64 {
        self.list.len() as u64 + self.more
    }

    /// The first error.
    pub fn first(&self) -> Option<&ScriptError> {
        self.list.first()
    }

    /// True for a script that could not be read (105) rather than checked (106).
    pub fn is_load(&self) -> bool {
        self.list.iter().any(|e| e.code.is_load())
    }

    /// The human lines (§1.8.7): one per error, then `… and <n> more` past the cap.
    pub fn lines(&self, script: &str) -> Vec<String> {
        let mut lines: Vec<String> = self.list.iter().map(|e| e.line(script)).collect();
        if self.more > 0 {
            lines.push(format!("… and {} more", self.more));
        }
        lines
    }
}
