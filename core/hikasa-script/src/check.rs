//! The check of a script before anything runs (§1.8.5, §1.8.7): the grammar, each word without `%` in its slot (the
//! script's own words here, every vocabulary word through the host's [`WordChecker`]), the flow check `no_comparison`, and
//! the references `%n` and `%NAME%`.

use std::collections::BTreeMap;
use std::ffi::OsString;

use crate::Errors;
use crate::ast::{Args, Command, CommandName, LoadArgs, Script, Slot};
use crate::error::{ScriptError, ScriptErrorCode};
use crate::expand::{References, normalize_sides, reference_errors};
use crate::lex::lex;
use crate::parse::parse_line;

/// Judges a vocabulary word in its slot, with the parser the CLI flag uses (T11). The word holds no `%` form and its
/// `lt`/`rt` are already normalized. `Err` is the message, which the check prints at the word.
pub trait WordChecker {
    /// `Ok` when the host takes `word` in `slot`.
    fn check(&self, slot: Slot, word: &str) -> Result<(), String>;
}

/// A checker that takes every word: the script's own syntax only (tests of the pure layer, tools that only lex).
#[derive(Debug, Clone, Copy, Default)]
pub struct AnyWord;

impl WordChecker for AnyWord {
    fn check(&self, _slot: Slot, _word: &str) -> Result<(), String> {
        Ok(())
    }
}

/// What the references are checked against before anything runs (§1.8.4).
#[derive(Debug, Clone, Copy)]
pub struct RefCheck<'a> {
    /// The arguments given; `None` (`script check` without arguments): `%n` is not checked.
    pub args: Option<&'a [OsString]>,
    /// The captured environment.
    pub env: &'a BTreeMap<String, OsString>,
    /// Names are found ASCII case-insensitively (Windows).
    pub env_case_insensitive: bool,
}

fn is(word: &str, kw: &str) -> bool {
    word.eq_ignore_ascii_case(kw)
}

/// The script's own judgement of a word of `slot` (sides, directions, `yes|no`, log levels), before the host's.
pub fn script_syntax(slot: Slot, word: &str) -> Result<(), String> {
    let side = |w: &str| is(w, "left") || is(w, "right");
    let direction = |w: &str| match w.split_once("->") {
        Some((from, to)) => side(from) && side(to) && !from.eq_ignore_ascii_case(to),
        None => false,
    };
    let ok = match slot {
        Slot::Side => side(word),
        Slot::Sides => side(word) || is(word, "all"),
        Slot::Direction => direction(word),
        Slot::SyncDirection => direction(word) || is(word, "all"),
        Slot::Recycle => is(word, "yes") || is(word, "no"),
        Slot::LogLevel => ["none", "normal", "verbose"]
            .into_iter()
            .any(|l| is(word, l)),
        _ => true,
    };
    if ok {
        return Ok(());
    }
    Err(match slot {
        Slot::Side => format!("`{word}` is not a side (left, right)"),
        Slot::Sides => {
            if word.contains("->") {
                format!("`{word}` is a direction where sides are wanted (left, right, all)")
            } else {
                format!("`{word}` is not a side (left, right, all)")
            }
        }
        Slot::Direction => format!("`{word}` is not a direction (left->right, right->left)"),
        Slot::SyncDirection => {
            format!("`{word}` is not a sync direction (left->right, right->left, all)")
        }
        Slot::Recycle => format!("recyclebin takes yes or no, not `{word}`"),
        _ => format!("`{word}` is not a log level (none, normal, verbose)"),
    })
}

/// Checks one word of `slot`: `lt`/`rt` normalized (unless `quoted`), the script's syntax, then the host's. `Ok` holds
/// the word the host gets.
pub fn check_word(
    slot: Slot,
    word: &str,
    quoted: bool,
    checker: &dyn WordChecker,
) -> Result<String, String> {
    let word = normalize_sides(slot, word, quoted);
    script_syntax(slot, &word)?;
    checker.check(slot, &word)?;
    Ok(word.into_owned())
}

/// The slot errors of one command: each word without `%` checked now; a word with `%` is checked when it ran (§1.8.4).
fn slot_errors(command: &Command, checker: &dyn WordChecker, errors: &mut Errors) {
    for (slot, word) in command.slots() {
        if word.expands {
            continue;
        }
        if let Err(message) = check_word(slot, &word.text, word.quoted, checker) {
            errors.push(ScriptError::new(
                word.pos,
                ScriptErrorCode::InvalidWord,
                message,
            ));
        }
    }
}

/// Where the flow check stands: a comparison loaded, none yet, or one dropped by `load <default>` at a line.
#[derive(Debug, Clone, Copy)]
enum Loaded {
    Yes,
    Never,
    Dropped(u32),
}

/// The flow check (§1.8.5): a command of column *Needs* before any load, or after `load <default>` without a later load.
fn flow(command: &Command, loaded: &mut Loaded, errors: &mut Errors) {
    if command.name.needs_comparison() {
        let why = match *loaded {
            Loaded::Yes => None,
            Loaded::Never => Some(format!("no load before line {}", command.pos.line)),
            Loaded::Dropped(line) => Some(format!("load <default> at line {line} dropped it")),
        };
        if let Some(why) = why {
            errors.push(ScriptError::new(
                command.pos,
                ScriptErrorCode::NoComparison,
                format!("{} needs a loaded comparison: {why}", command.name.as_str()),
            ));
        }
    }
    if let Args::Load(load) = &command.args {
        *loaded = match load {
            LoadArgs::Default => Loaded::Dropped(command.pos.line),
            LoadArgs::Session(_) | LoadArgs::Folders { .. } => Loaded::Yes,
        };
    }
}

/// Lexes, parses and checks `source` with `checker` (§1.8.3–§1.8.7), without the reference checks.
pub fn parse(source: &[u8], checker: &dyn WordChecker) -> Result<Script, Errors> {
    parse_checked(source, checker, None)
}

/// [`parse`], and with `refs` the references: a `%n` above the arguments given, a `%NAME%` not set. Every error is
/// reported in line order (up to [`crate::SCRIPT_MAX_ERRORS`], the rest counted).
pub fn parse_checked(
    source: &[u8],
    checker: &dyn WordChecker,
    refs: Option<&RefCheck<'_>>,
) -> Result<Script, Errors> {
    let lines = lex(source)?;
    let mut errors = Errors::default();
    let mut commands = Vec::with_capacity(lines.len());
    let mut references = References::default();
    let mut loaded = Loaded::Never;
    for line in &lines {
        for word in &line.words {
            references.add(word);
        }
        if let Some(refs) = refs {
            for error in reference_errors(
                line.words.iter(),
                refs.args.map(<[OsString]>::len),
                refs.env,
                refs.env_case_insensitive,
            ) {
                errors.push(error);
            }
        }
        match parse_line(line) {
            Ok(command) => {
                slot_errors(&command, checker, &mut errors);
                flow(&command, &mut loaded, &mut errors);
                if errors.is_empty() {
                    commands.push(command);
                }
            }
            Err(error) => {
                errors.push(error);
                // The flow goes on as if the line were a load when it names one, so one typo gives one error.
                if line
                    .words
                    .first()
                    .is_some_and(|w| CommandName::parse(&w.text) == Some(CommandName::Load))
                {
                    loaded = Loaded::Yes;
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(Script {
            commands,
            references,
        })
    } else {
        Err(errors)
    }
}
