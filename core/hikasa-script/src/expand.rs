//! `%`-expansion (§1.8.4, D-217): per word, after the line is split into words and just before its command runs; a value
//! is never lexed again. And `lt`/`rt` in side positions (D-218).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};

use serde::Serialize;

use crate::ast::Slot;
use crate::error::{ScriptError, ScriptErrorCode};
use crate::lex::Word;

/// A local date and time, to the second (the Service's clock in the user's zone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
pub struct LocalTime {
    /// The year.
    pub year: i16,
    /// 1…12.
    pub month: u8,
    /// 1…31.
    pub day: u8,
    /// 0…23.
    pub hour: u8,
    /// 0…59.
    pub minute: u8,
    /// 0…59 (60 for a leap second, where a clock gives one).
    pub second: u8,
}

impl LocalTime {
    /// `YYYY-MM-DD`.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `HH:MM:SS`.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `HH-MM-SS`.
    pub fn fn_time(&self) -> String {
        format!("{:02}-{:02}-{:02}", self.hour, self.minute, self.second)
    }
}

/// What expansion reads.
#[derive(Debug, Clone, Copy)]
pub struct ExpandContext<'a> {
    /// `%0`: the script as located.
    pub script: &'a OsStr,
    /// `%1`…`%9`.
    pub args: &'a [OsString],
    /// The captured environment: only the names the script references.
    pub env: &'a BTreeMap<String, OsString>,
    /// Names are found ASCII case-insensitively (Windows).
    pub env_case_insensitive: bool,
    /// The instant of this command.
    pub now: LocalTime,
}

/// One piece of a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    /// Text as written.
    Literal(&'a str),
    /// `%%`.
    Percent,
    /// `%0`…`%9`.
    Arg(u8),
    /// `%date%`.
    Date,
    /// `%time%`.
    Time,
    /// `%fn_time%`.
    FnTime,
    /// `%NAME%`.
    Var(&'a str),
}

/// True for `[A-Za-z_][A-Za-z0-9_]*`.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The pieces of `text`, left to right.
pub(crate) fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut literal_from = 0;
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < bytes.len() {
        if bytes[at] != b'%' {
            at += 1;
            continue;
        }
        let next = bytes.get(at + 1).copied();
        let (piece, len) = match next {
            Some(b'%') => (Some(Piece::Percent), 2),
            Some(d @ b'0'..=b'9') => (Some(Piece::Arg(d - b'0')), 2),
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => match text[at + 1..].find('%') {
                Some(end) if is_name(&text[at + 1..at + 1 + end]) => {
                    let name = &text[at + 1..at + 1 + end];
                    let piece = if name.eq_ignore_ascii_case("date") {
                        Piece::Date
                    } else if name.eq_ignore_ascii_case("time") {
                        Piece::Time
                    } else if name.eq_ignore_ascii_case("fn_time") {
                        Piece::FnTime
                    } else {
                        Piece::Var(name)
                    };
                    (Some(piece), end + 2)
                }
                _ => (None, 1),
            },
            _ => (None, 1),
        };
        match piece {
            Some(piece) => {
                if literal_from < at {
                    out.push(Piece::Literal(&text[literal_from..at]));
                }
                out.push(piece);
                at += len;
                literal_from = at;
            }
            None => at += len,
        }
    }
    if literal_from < bytes.len() {
        out.push(Piece::Literal(&text[literal_from..]));
    }
    out
}

/// True when `text` holds a form that expansion replaces (`%%` included).
pub fn has_forms(text: &str) -> bool {
    text.contains('%')
        && pieces(text)
            .iter()
            .any(|piece| !matches!(piece, Piece::Literal(_)))
}

/// The value of `name` in `env`.
fn lookup<'a>(
    env: &'a BTreeMap<String, OsString>,
    name: &str,
    case_insensitive: bool,
) -> Option<&'a OsString> {
    env.get(name).or_else(|| {
        case_insensitive
            .then(|| {
                env.iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value)
            })
            .flatten()
    })
}

/// The `missing_script_argument` error of `%n` with `given` arguments.
pub(crate) fn missing_argument(word: &Word, n: u8, given: usize) -> ScriptError {
    let given_text = match given {
        1 => "1 argument was".to_owned(),
        n => format!("{n} arguments were"),
    };
    ScriptError::new(
        word.pos,
        ScriptErrorCode::MissingScriptArgument,
        format!("the script uses %{n}, but {given_text} given"),
    )
}

/// The `unset_variable` error of `%NAME%`.
pub(crate) fn unset_variable(word: &Word, name: &str) -> ScriptError {
    ScriptError::new(
        word.pos,
        ScriptErrorCode::UnsetVariable,
        format!("the environment variable {name} is not set"),
    )
}

/// Expands `word` (§1.8.4): `%1`…`%9`, `%0`, `%%`, the dynamic names, `%NAME%`; any other `%` is itself. A missing
/// argument or an unset variable is an error at the word (normally found before anything runs).
pub fn expand(word: &Word, ctx: &ExpandContext<'_>) -> Result<OsString, ScriptError> {
    if !word.expands {
        return Ok(OsString::from(&word.text));
    }
    let mut out = OsString::with_capacity(word.text.len());
    for piece in pieces(&word.text) {
        match piece {
            Piece::Literal(text) => out.push(text),
            Piece::Percent => out.push("%"),
            Piece::Arg(0) => out.push(ctx.script),
            Piece::Arg(n) => match ctx.args.get(usize::from(n) - 1) {
                Some(value) => out.push(value),
                None => return Err(missing_argument(word, n, ctx.args.len())),
            },
            Piece::Date => out.push(ctx.now.date()),
            Piece::Time => out.push(ctx.now.time()),
            Piece::FnTime => out.push(ctx.now.fn_time()),
            Piece::Var(name) => match lookup(ctx.env, name, ctx.env_case_insensitive) {
                Some(value) => out.push(value),
                None => return Err(unset_variable(word, name)),
            },
        }
    }
    Ok(out)
}

/// What a script references (§1.8.4): the highest `%n`, the environment names, `%0`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct References {
    /// The highest `%n` (0: none).
    pub args: u8,
    /// The names of `%NAME%`, as written.
    pub env: BTreeSet<String>,
    /// `%0` is used.
    pub uses_script_path: bool,
}

impl References {
    /// Adds the forms of `word`.
    pub(crate) fn add(&mut self, word: &Word) {
        if !word.expands {
            return;
        }
        for piece in pieces(&word.text) {
            match piece {
                Piece::Arg(0) => self.uses_script_path = true,
                Piece::Arg(n) => self.args = self.args.max(n),
                Piece::Var(name) => {
                    self.env.insert(name.to_owned());
                }
                Piece::Literal(_) | Piece::Percent | Piece::Date | Piece::Time | Piece::FnTime => {}
            }
        }
    }
}

/// The errors of `words` before anything runs: a `%n` above `args` (when given) and an unset variable.
pub(crate) fn reference_errors<'w>(
    words: impl Iterator<Item = &'w Word>,
    args: Option<usize>,
    env: &BTreeMap<String, OsString>,
    case_insensitive: bool,
) -> Vec<ScriptError> {
    let mut errors = Vec::new();
    for word in words.filter(|word| word.expands) {
        for piece in pieces(&word.text) {
            match piece {
                Piece::Arg(n) if n > 0 => {
                    if let Some(given) = args
                        && usize::from(n) > given
                    {
                        errors.push(missing_argument(word, n, given));
                    }
                }
                Piece::Var(name) if lookup(env, name, case_insensitive).is_none() => {
                    errors.push(unset_variable(word, name));
                }
                _ => {}
            }
        }
    }
    errors
}

/// `left` and `right` for `lt` and `rt` (ASCII case-insensitive), other words as they are.
fn side_word(word: &str) -> Cow<'_, str> {
    if word.eq_ignore_ascii_case("lt") {
        Cow::Borrowed("left")
    } else if word.eq_ignore_ascii_case("rt") {
        Cow::Borrowed("right")
    } else {
        Cow::Borrowed(word)
    }
}

/// `lt` and `rt` as `left` and `right` where `slot` expects a side (D-218): a side or sides word, both ends of a
/// direction, the side part of a select mask, the side of `timezone:` in criteria. Every other slot, and a word that was
/// quoted, is returned as it is.
pub fn normalize_sides<'a>(slot: Slot, word: &'a str, quoted: bool) -> Cow<'a, str> {
    if quoted || !word.is_ascii() {
        return Cow::Borrowed(word);
    }
    match slot {
        Slot::Side | Slot::Sides => side_word(word),
        Slot::Direction | Slot::SyncDirection => match word.split_once("->") {
            Some((from, to)) => Cow::Owned(format!("{}->{}", side_word(from), side_word(to))),
            None => Cow::Borrowed(word),
        },
        Slot::SelectMask => match word.split_once('.') {
            Some((side, rest))
                if side.eq_ignore_ascii_case("lt") || side.eq_ignore_ascii_case("rt") =>
            {
                Cow::Owned(format!("{}.{rest}", side_word(side)))
            }
            Some(_) => Cow::Borrowed(word),
            None => side_word(word),
        },
        Slot::Criteria => {
            let lower = word.to_ascii_lowercase();
            let mut out = String::with_capacity(word.len());
            let mut changed = false;
            let mut from = 0;
            for (at, _) in lower.match_indices("timezone:") {
                let starts_item =
                    at == 0 || lower[..at].ends_with(|c: char| c == ',' || c.is_ascii_whitespace());
                let value = at + "timezone:".len();
                let side = &lower[value..lower.len().min(value + 2)];
                if starts_item && (side == "lt" || side == "rt") {
                    out.push_str(&word[from..value]);
                    out.push_str(if side == "lt" { "left" } else { "right" });
                    from = value + 2;
                    changed = true;
                }
            }
            if changed {
                out.push_str(&word[from..]);
                Cow::Owned(out)
            } else {
                Cow::Borrowed(word)
            }
        }
        _ => Cow::Borrowed(word),
    }
}
