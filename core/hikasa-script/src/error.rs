//! Positions and the errors of a script's check (§1.8.7, D-222).

use serde::Serialize;

/// A place in a script: 1-based line and column; the column counts Unicode scalar values, a tab is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Pos {
    /// The physical line, from 1.
    pub line: u32,
    /// The column, from 1.
    pub column: u32,
}

impl Pos {
    /// The first character of a script.
    pub const START: Pos = Pos { line: 1, column: 1 };

    /// `line`, `column`.
    pub const fn new(line: u32, column: u32) -> Pos {
        Pos { line, column }
    }
}

/// What is wrong with a script before anything runs (106), or that it is too large to read (105).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptErrorCode {
    /// The file is larger than [`crate::SCRIPT_MAX_BYTES`] (exit 105, nothing lexed).
    TooLarge,
    /// A byte that is not UTF-8.
    InvalidUtf8,
    /// A quote that does not close on its physical line.
    UnterminatedQuote,
    /// A `&` before a blank line or the end of the file.
    ContinuationAtEnd,
    /// A command name the language does not know.
    UnknownCommand,
    /// A word the command needs is missing.
    MissingArgument,
    /// A word the command does not take.
    UnexpectedArgument,
    /// A keyword given twice.
    DuplicateKeyword,
    /// A keyword of another command.
    UnknownKeyword,
    /// A word its slot refuses.
    InvalidWord,
    /// A command that needs a loaded comparison before any load.
    NoComparison,
    /// A `%n` above the number of arguments given.
    MissingScriptArgument,
    /// A `%NAME%` that is not set.
    UnsetVariable,
}

impl ScriptErrorCode {
    /// Every code, in the order of §1.8.7 (`too_large` first: it is the load's).
    pub const ALL: [ScriptErrorCode; 13] = [
        ScriptErrorCode::TooLarge,
        ScriptErrorCode::InvalidUtf8,
        ScriptErrorCode::UnterminatedQuote,
        ScriptErrorCode::ContinuationAtEnd,
        ScriptErrorCode::UnknownCommand,
        ScriptErrorCode::MissingArgument,
        ScriptErrorCode::UnexpectedArgument,
        ScriptErrorCode::DuplicateKeyword,
        ScriptErrorCode::UnknownKeyword,
        ScriptErrorCode::InvalidWord,
        ScriptErrorCode::NoComparison,
        ScriptErrorCode::MissingScriptArgument,
        ScriptErrorCode::UnsetVariable,
    ];

    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ScriptErrorCode::TooLarge => "too_large",
            ScriptErrorCode::InvalidUtf8 => "invalid_utf8",
            ScriptErrorCode::UnterminatedQuote => "unterminated_quote",
            ScriptErrorCode::ContinuationAtEnd => "continuation_at_end",
            ScriptErrorCode::UnknownCommand => "unknown_command",
            ScriptErrorCode::MissingArgument => "missing_argument",
            ScriptErrorCode::UnexpectedArgument => "unexpected_argument",
            ScriptErrorCode::DuplicateKeyword => "duplicate_keyword",
            ScriptErrorCode::UnknownKeyword => "unknown_keyword",
            ScriptErrorCode::InvalidWord => "invalid_word",
            ScriptErrorCode::NoComparison => "no_comparison",
            ScriptErrorCode::MissingScriptArgument => "missing_script_argument",
            ScriptErrorCode::UnsetVariable => "unset_variable",
        }
    }

    /// True for the code of a script that could not be read (105); every other is a check error (106).
    pub const fn is_load(self) -> bool {
        matches!(self, ScriptErrorCode::TooLarge)
    }
}

/// One error, at its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{}:{}: error: {message}", pos.line, pos.column)]
pub struct ScriptError {
    /// Where.
    pub pos: Pos,
    /// What.
    pub code: ScriptErrorCode,
    /// The fixed English message.
    pub message: String,
}

impl ScriptError {
    /// An error.
    pub fn new(pos: Pos, code: ScriptErrorCode, message: impl Into<String>) -> ScriptError {
        ScriptError {
            pos,
            code,
            message: message.into(),
        }
    }

    /// The human line: `<script>:<line>:<column>: error: <message>` (§1.8.7).
    pub fn line(&self, script: &str) -> String {
        format!(
            "{script}:{}:{}: error: {}",
            self.pos.line, self.pos.column, self.message
        )
    }
}
