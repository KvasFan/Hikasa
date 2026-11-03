//! The lexer (§1.8.3, D-216): bytes become logical command lines of words. Total: any byte string gives lines or errors,
//! never a panic (P-LEX).

use serde::Serialize;

use crate::error::{Pos, ScriptError, ScriptErrorCode};
use crate::{Errors, SCRIPT_MAX_BYTES};

/// One word of a command line: quotes removed, `%` forms not expanded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Word {
    /// The text, quotes removed (`""` inside quotes is one `"`).
    pub text: String,
    /// Any part of the word was quoted.
    pub quoted: bool,
    /// The text holds a `%` form that expansion replaces (§1.8.4).
    pub expands: bool,
    /// The line and column of its first character (an opening quote is its first character).
    pub pos: Pos,
}

impl Word {
    /// A word at `pos`; `expands` is computed from the text.
    pub fn new(text: impl Into<String>, quoted: bool, pos: Pos) -> Word {
        let text = text.into();
        Word {
            expands: crate::expand::has_forms(&text),
            text,
            quoted,
            pos,
        }
    }

    /// The word as a script writes it: quoted when it was quoted or when its text needs it (empty, a blank, a quote, a
    /// leading `#`, a trailing `&`), `"` doubled inside the quotes. Lexing the result gives this word's text back.
    pub fn to_source(&self) -> String {
        let needs = self.quoted
            || self.text.is_empty()
            || self.text.contains([' ', '\t', '"'])
            || self.text.starts_with('#')
            || self.text.ends_with('&');
        if needs {
            format!("\"{}\"", self.text.replace('"', "\"\""))
        } else {
            self.text.clone()
        }
    }
}

/// One logical command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    /// The position of its first word.
    pub pos: Pos,
    /// Its last physical line.
    pub end_line: u32,
    /// The words, the command name first.
    pub words: Vec<Word>,
}

/// What one physical line holds.
struct Physical {
    words: Vec<Word>,
    /// The position of a trailing `&` outside quotes (already removed from the words).
    amp: Option<Pos>,
    /// A comment was found.
    comment: bool,
    /// An unterminated quote, at the opening quote.
    unterminated: Option<Pos>,
}

/// A word being built.
struct Building {
    text: String,
    quoted: bool,
    pos: Pos,
    /// The last character added was an unquoted `&`, at this position.
    amp: Option<Pos>,
}

/// Lexes one physical line, numbered `line`; with `continuation`, a trailing unquoted `&` continues the command.
fn lex_physical(text: &str, line: u32, continuation: bool) -> Physical {
    let mut words = Vec::new();
    let mut current: Option<Building> = None;
    let mut quote: Option<Pos> = None;
    let mut comment = false;
    let mut column: u32 = 0;
    let mut chars = text.chars().peekable();
    let finish = |current: &mut Option<Building>, built: &mut Vec<Building>| {
        if let Some(word) = current.take() {
            built.push(word);
        }
    };
    let mut built: Vec<Building> = Vec::new();
    while let Some(c) = chars.next() {
        column = column.saturating_add(1);
        let here = Pos::new(line, column);
        if quote.is_some() {
            let word = current.get_or_insert_with(|| Building {
                text: String::new(),
                quoted: true,
                pos: here,
                amp: None,
            });
            word.amp = None;
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    column = column.saturating_add(1);
                    word.text.push('"');
                } else {
                    quote = None;
                }
            } else {
                word.text.push(c);
            }
            continue;
        }
        match c {
            ' ' | '\t' => finish(&mut current, &mut built),
            '#' if current.is_none() => {
                comment = true;
                break;
            }
            '"' => {
                let word = current.get_or_insert_with(|| Building {
                    text: String::new(),
                    quoted: true,
                    pos: here,
                    amp: None,
                });
                word.quoted = true;
                word.amp = None;
                quote = Some(here);
            }
            _ => {
                let word = current.get_or_insert_with(|| Building {
                    text: String::new(),
                    quoted: false,
                    pos: here,
                    amp: None,
                });
                word.text.push(c);
                word.amp = (c == '&').then_some(here);
            }
        }
    }
    let unterminated = quote;
    finish(&mut current, &mut built);
    let mut amp = None;
    if continuation
        && unterminated.is_none()
        && let Some(last) = built.last_mut()
        && let Some(at) = last.amp
    {
        amp = Some(at);
        last.text.pop();
        if last.text.is_empty() && !last.quoted {
            built.pop();
        }
    }
    words.extend(
        built
            .into_iter()
            .map(|word| Word::new(word.text, word.quoted, word.pos)),
    );
    Physical {
        words,
        amp,
        comment,
        unterminated,
    }
}

/// The physical lines of `text` with their numbers: LF, CRLF and CR each end one.
fn physical_lines(text: &str) -> impl Iterator<Item = (u32, &str)> {
    let mut rest = Some(text);
    let mut number: u32 = 0;
    std::iter::from_fn(move || {
        let current = rest?;
        number = number.saturating_add(1);
        match current.find(['\n', '\r']) {
            Some(at) => {
                let line = &current[..at];
                let after = &current[at..];
                let skip = if after.starts_with("\r\n") { 2 } else { 1 };
                rest = Some(&after[skip..]);
                Some((number, line))
            }
            None => {
                rest = None;
                // A file that ends with a line end has no further line.
                if current.is_empty() && number > 1 {
                    None
                } else {
                    Some((number, current))
                }
            }
        }
    })
}

/// The text of `body`, or the error at its first byte that is not UTF-8 (its line, and the column after the scalar
/// before it).
fn decode(body: &[u8]) -> Result<&str, ScriptError> {
    match std::str::from_utf8(body) {
        Ok(text) => Ok(text),
        Err(e) => {
            let valid = &body[..e.valid_up_to()];
            // Valid by construction; a failure would be a bug, and is reported at the start rather than a panic.
            let before = std::str::from_utf8(valid).unwrap_or("");
            let mut line: u32 = 1;
            let mut column: u32 = 0;
            let mut chars = before.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '\r' => {
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                        line = line.saturating_add(1);
                        column = 0;
                    }
                    '\n' => {
                        line = line.saturating_add(1);
                        column = 0;
                    }
                    _ => column = column.saturating_add(1),
                }
            }
            Err(ScriptError::new(
                Pos::new(line, column.saturating_add(1)),
                ScriptErrorCode::InvalidUtf8,
                "the script is not valid UTF-8 here",
            ))
        }
    }
}

/// The error of a script over [`SCRIPT_MAX_BYTES`].
pub fn too_large(len: u64) -> ScriptError {
    ScriptError::new(
        Pos::START,
        ScriptErrorCode::TooLarge,
        format!("the script is {len} bytes, more than the 16 MiB a script may hold"),
    )
}

/// Lexes a script (§1.8.3): the logical lines with their words, or every lexical error (up to
/// [`crate::SCRIPT_MAX_ERRORS`], the rest counted). A file over [`SCRIPT_MAX_BYTES`] is `too_large` and nothing is lexed;
/// a byte that is not UTF-8 is one `invalid_utf8` error at the first such byte.
pub fn lex(source: &[u8]) -> Result<Vec<Line>, Errors> {
    let mut lines = Vec::new();
    let errors = lex_each(source, &mut |line| lines.push(line));
    if errors.is_empty() {
        Ok(lines)
    } else {
        Err(errors)
    }
}

/// [`lex`], giving each logical line to `each` as it ends instead of keeping them: a large script is never held as lines
/// (G14). Once an error is found no further line is given (a script with an error has no lines); the errors are
/// returned, empty when it lexed.
pub(crate) fn lex_each(source: &[u8], each: &mut dyn FnMut(Line)) -> Errors {
    let mut errors = Errors::default();
    if source.len() as u64 > SCRIPT_MAX_BYTES {
        errors.push(too_large(source.len() as u64));
        return errors;
    }
    let body = source.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(source);
    let text = match decode(body) {
        Ok(text) => text,
        Err(error) => {
            errors.push(error);
            return errors;
        }
    };
    // A command continued by `&`: its words so far and where the `&` was.
    let mut pending: Option<(Vec<Word>, Pos)> = None;
    for (number, physical) in physical_lines(text) {
        let lexed = lex_physical(physical, number, true);
        if let Some(at) = lexed.unterminated {
            errors.push(ScriptError::new(
                at,
                ScriptErrorCode::UnterminatedQuote,
                "this quote does not close on its line",
            ));
            pending = None;
            continue;
        }
        let mut words = match pending.take() {
            Some((words, amp)) => {
                if lexed.words.is_empty() && lexed.amp.is_none() {
                    if lexed.comment {
                        // A comment-only line between continued lines is skipped.
                        pending = Some((words, amp));
                    } else {
                        errors.push(continuation_at_end(amp));
                    }
                    continue;
                }
                words
            }
            None => Vec::new(),
        };
        words.extend(lexed.words);
        match lexed.amp {
            Some(amp) => pending = Some((words, amp)),
            None => {
                if errors.is_empty()
                    && let Some(first) = words.first()
                {
                    each(Line {
                        pos: first.pos,
                        end_line: number,
                        words,
                    });
                }
            }
        }
    }
    if let Some((_, amp)) = pending {
        errors.push(continuation_at_end(amp));
    }
    errors
}

fn continuation_at_end(amp: Pos) -> ScriptError {
    ScriptError::new(
        amp,
        ScriptErrorCode::ContinuationAtEnd,
        "this `&` continues the command, but a blank line or the end of the script follows",
    )
}

/// Splits one line of arguments by the lexer's word rules (the Run Script dialog's field; no expansion, no comments
/// across lines): the words' texts, or the first lexical error.
pub fn split_words(text: &str) -> Result<Vec<String>, ScriptError> {
    if let Some(at) = text.find(['\n', '\r']) {
        let column = text[..at].chars().count() as u32 + 1;
        return Err(ScriptError::new(
            Pos::new(1, column),
            ScriptErrorCode::InvalidWord,
            "the arguments are one line",
        ));
    }
    let lexed = lex_physical(text, 1, false);
    if let Some(at) = lexed.unterminated {
        return Err(ScriptError::new(
            at,
            ScriptErrorCode::UnterminatedQuote,
            "this quote does not close on its line",
        ));
    }
    Ok(lexed.words.into_iter().map(|word| word.text).collect())
}
