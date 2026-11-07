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
///
/// It walks the bytes: the characters it acts on (blank, tab, `#`, `"`, `&`) are ASCII, and a run of other characters is
/// added to its word as one slice, its columns counted by the bytes that start a character (a 16 MiB script checks in
/// a debug build within A8.15's bound; `tests::fast_lexer_is_the_char_lexer` holds it to the char-by-char lexer).
fn lex_physical(text: &str, line: u32, continuation: bool) -> Physical {
    let bytes = text.as_bytes();
    let mut built: Vec<Word> = Vec::new();
    // Where the last word built ends with an unquoted `&`.
    let mut last_amp: Option<Pos> = None;
    let mut current: Option<Building> = None;
    let mut quote: Option<Pos> = None;
    let mut comment = false;
    // The column of the last character taken.
    let mut column: u32 = 0;
    let mut i = 0;
    let start_word = |current: &mut Option<Building>, quoted: bool, pos: Pos| {
        if current.is_none() {
            *current = Some(Building {
                text: String::new(),
                quoted,
                pos,
                amp: None,
            });
        }
    };
    while i < bytes.len() {
        let b = bytes[i];
        let quoted = quote.is_some();
        if quoted && b == b'"' {
            column = column.saturating_add(1);
            start_word(&mut current, true, Pos::new(line, column));
            if bytes.get(i + 1) == Some(&b'"') {
                column = column.saturating_add(1);
                if let Some(word) = &mut current {
                    word.text.push('"');
                    word.amp = None;
                }
                i += 2;
            } else {
                quote = None;
                if let Some(word) = &mut current {
                    word.amp = None;
                }
                i += 1;
            }
            continue;
        }
        if !quoted {
            match b {
                b' ' | b'\t' => {
                    column = column.saturating_add(1);
                    if let Some(word) = current.take() {
                        last_amp = word.amp;
                        built.push(Word::new(word.text, word.quoted, word.pos));
                    }
                    i += 1;
                    continue;
                }
                b'#' if current.is_none() => {
                    comment = true;
                    break;
                }
                b'"' => {
                    column = column.saturating_add(1);
                    let here = Pos::new(line, column);
                    start_word(&mut current, true, here);
                    if let Some(word) = &mut current {
                        word.quoted = true;
                        word.amp = None;
                    }
                    quote = Some(here);
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        // A run of characters the word takes as they are: up to a `"`, or outside quotes up to a blank or a tab.
        let run = i;
        let first = column.saturating_add(1);
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'"' || (!quoted && (b == b' ' || b == b'\t')) {
                break;
            }
            if b & 0xC0 != 0x80 {
                column = column.saturating_add(1);
            }
            i += 1;
        }
        start_word(&mut current, quoted, Pos::new(line, first));
        if let Some(word) = &mut current {
            word.text.push_str(&text[run..i]);
            // Outside quotes an `&` that ends the run is a continuation candidate; inside, never.
            word.amp = (!quoted && bytes[i - 1] == b'&').then_some(Pos::new(line, column));
        }
    }
    let unterminated = quote;
    if let Some(word) = current.take() {
        last_amp = word.amp;
        built.push(Word::new(word.text, word.quoted, word.pos));
    }
    let mut amp = None;
    if continuation
        && unterminated.is_none()
        && let Some(at) = last_amp
        && let Some(last) = built.last_mut()
    {
        amp = Some(at);
        last.text.pop();
        if last.text.is_empty() && !last.quoted {
            built.pop();
        } else {
            last.expands = crate::expand::has_forms(&last.text);
        }
    }
    Physical {
        words: built,
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
        let bytes = current.as_bytes();
        let mut at = 0;
        while at < bytes.len() && bytes[at] != b'\n' && bytes[at] != b'\r' {
            at += 1;
        }
        match (at < bytes.len()).then_some(at) {
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
    let mut errors = Errors::default();
    lex_each(source, &mut |lexed| match lexed {
        Lexed::Line(line) if errors.is_empty() => lines.push(line),
        Lexed::Line(_) => {}
        Lexed::Error { error, .. } => errors.push(error),
    });
    if errors.is_empty() {
        Ok(lines)
    } else {
        Err(errors)
    }
}

/// What [`lex_each`] gives, in the order of the source.
pub(crate) enum Lexed {
    /// A logical line that lexed.
    Line(Line),
    /// A lexical error; the logical line it broke is dropped and lexing goes on at the next one.
    Error {
        /// The error.
        error: ScriptError,
        /// The first word of the broken logical line, when it has one (the flow check reads a broken `load` as a load).
        first: Option<String>,
    },
}

/// [`lex`], giving each logical line to `each` as it ends instead of keeping them: a large script is never held as lines
/// (G14). A lexical error drops its logical line and lexing goes on at the next one, so the checks of the lines around it
/// still report (§1.8.7: every error); a file over the cap or not UTF-8 gives its one error and nothing else.
pub(crate) fn lex_each(source: &[u8], each: &mut dyn FnMut(Lexed)) {
    if source.len() as u64 > SCRIPT_MAX_BYTES {
        each(Lexed::Error {
            error: too_large(source.len() as u64),
            first: None,
        });
        return;
    }
    let body = source.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(source);
    let text = match decode(body) {
        Ok(text) => text,
        Err(error) => {
            each(Lexed::Error { error, first: None });
            return;
        }
    };
    // A command continued by `&`: its words so far and where the `&` was.
    let mut pending: Option<(Vec<Word>, Pos)> = None;
    for (number, physical) in physical_lines(text) {
        let lexed = lex_physical(physical, number, true);
        if let Some(at) = lexed.unterminated {
            let first = match pending.take() {
                Some((words, _)) => words.into_iter().next(),
                None => lexed.words.into_iter().next(),
            };
            each(Lexed::Error {
                error: ScriptError::new(
                    at,
                    ScriptErrorCode::UnterminatedQuote,
                    "this quote does not close on its line",
                ),
                first: first.map(|w| w.text),
            });
            continue;
        }
        let words = match pending.take() {
            Some((mut words, amp)) => {
                if lexed.words.is_empty() && lexed.amp.is_none() {
                    if lexed.comment {
                        // A comment-only line between continued lines is skipped.
                        pending = Some((words, amp));
                    } else {
                        each(Lexed::Error {
                            error: continuation_at_end(amp),
                            first: words.into_iter().next().map(|w| w.text),
                        });
                    }
                    continue;
                }
                words.extend(lexed.words);
                words
            }
            None => lexed.words,
        };
        match lexed.amp {
            Some(amp) => pending = Some((words, amp)),
            None => {
                if let Some(first) = words.first() {
                    each(Lexed::Line(Line {
                        pos: first.pos,
                        end_line: number,
                        words,
                    }));
                }
            }
        }
    }
    if let Some((words, amp)) = pending {
        each(Lexed::Error {
            error: continuation_at_end(amp),
            first: words.into_iter().next().map(|w| w.text),
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The char-by-char lexer the byte lexer replaced (the oracle of `fast_lexer_is_the_char_lexer`): lexes one physical line, numbered `line`; with `continuation`, a trailing unquoted `&` continues the command.
    fn char_lex_physical(text: &str, line: u32, continuation: bool) -> Physical {
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

    fn same(text: &str) {
        for continuation in [true, false] {
            let fast = lex_physical(text, 7, continuation);
            let slow = char_lex_physical(text, 7, continuation);
            assert_eq!(fast.words, slow.words, "{text:?}");
            assert_eq!(fast.amp, slow.amp, "{text:?}");
            assert_eq!(fast.comment, slow.comment, "{text:?}");
            assert_eq!(fast.unterminated, slow.unterminated, "{text:?}");
        }
    }

    #[test]
    fn fast_lexer_is_the_char_lexer_on_edges() {
        for text in [
            "",
            " ",
            "&",
            " & ",
            "a&",
            "a &",
            "\"&\"",
            "\"a\"&",
            "a\"b c\"d&",
            "\"\"",
            "\"\"\"\"",
            "\"a\"\"b\"",
            "\"open",
            "#c",
            " # c",
            "a#b",
            "a #b",
            "é&",
            "\"é\"\"ü\" ñ&",
            "x\t\ty",
            "&&",
            "a\"\"&",
            "\"&",
        ] {
            same(text);
        }
    }

    proptest! {
        #[test]
        fn fast_lexer_is_the_char_lexer(text in "[ \t#\"&a%é\u{1F600}]{0,24}") {
            same(&text);
        }
    }
}
