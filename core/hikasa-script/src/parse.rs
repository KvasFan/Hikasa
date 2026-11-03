//! The grammar of §1.8.6 (D-219): a lexed line becomes a [`Command`] with typed slots, or the errors of its arity and
//! keywords. Vocabulary words are not judged here (see `check`).

use crate::ast::{
    Args, Command, CommandName, CopyToArgs, ExpandArgs, FilterArgs, LoadArgs, LogArgs, OptionArgs,
    RenameArgs, ReportArgs, ReportFor, SyncArgs, SyncMode, TouchArgs,
};
use crate::error::{ScriptError, ScriptErrorCode};
use crate::lex::{Line, Word};

/// Every keyword word of the language (the part before the value, ASCII case-insensitive).
pub const KEYWORDS: [&str; 18] = [
    "create:",
    "path:",
    "recyclebin=",
    "recyclebin:",
    "layout:",
    "options:",
    "output-options:",
    "title:",
    "output-to:",
    "confirm:",
    "update:",
    "mirror:",
    "append:",
    "cutoff:",
    "size:",
    "attrib:",
    "unixtype:",
    "output:",
];

/// The value of `word` after the keyword `kw` (ASCII case-insensitive), as a word at the same place.
pub(crate) fn keyword(word: &Word, kw: &str) -> Option<Word> {
    starts_with_keyword(word, kw).then(|| Word::new(&word.text[kw.len()..], word.quoted, word.pos))
}

/// True when `word` starts with the keyword `kw` (ASCII case-insensitive), without making its value.
fn starts_with_keyword(word: &Word, kw: &str) -> bool {
    word.text
        .as_bytes()
        .get(..kw.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(kw.as_bytes()))
}

/// The keyword `word` starts with, if any.
fn any_keyword(word: &Word) -> Option<&'static str> {
    if !word.text.contains([':', '=']) {
        return None;
    }
    KEYWORDS
        .into_iter()
        .find(|kw| starts_with_keyword(word, kw))
}

/// True when `word` is `kw` (ASCII case-insensitive).
fn is(word: &Word, kw: &str) -> bool {
    word.text.eq_ignore_ascii_case(kw)
}

/// True for a side or sides word, `lt`/`rt` included.
pub(crate) fn is_side_word(text: &str) -> bool {
    ["left", "right", "all", "lt", "rt"]
        .into_iter()
        .any(|side| text.eq_ignore_ascii_case(side))
}

fn err(word: &Word, code: ScriptErrorCode, message: String) -> ScriptError {
    ScriptError::new(word.pos, code, message)
}

fn missing(at: &Word, name: CommandName, what: &str) -> ScriptError {
    err(
        at,
        ScriptErrorCode::MissingArgument,
        format!("{} needs {what}", name.as_str()),
    )
}

fn unexpected(word: &Word, name: CommandName) -> ScriptError {
    err(
        word,
        ScriptErrorCode::UnexpectedArgument,
        format!("{} does not take `{}` here", name.as_str(), word.text),
    )
}

fn duplicate(word: &Word, kw: &str) -> ScriptError {
    err(
        word,
        ScriptErrorCode::DuplicateKeyword,
        format!("`{kw}` is given twice"),
    )
}

fn foreign(word: &Word, kw: &str, name: CommandName) -> ScriptError {
    err(
        word,
        ScriptErrorCode::UnknownKeyword,
        format!(
            "`{kw}` is not a keyword of {} (a path that starts so is written `./{}`)",
            name.as_str(),
            word.text
        ),
    )
}

fn invalid(word: &Word, message: String) -> ScriptError {
    err(word, ScriptErrorCode::InvalidWord, message)
}

/// The edit distance of `a` and `b` (ASCII case-insensitive).
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<u8> = a.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let b: Vec<u8> = b.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous + usize::from(ca != cb);
            previous = row[j + 1];
            row[j + 1] = substitution.min(previous + 1).min(row[j] + 1);
        }
    }
    row[b.len()]
}

/// Up to three command names near `word`, nearest first.
pub fn suggestions(word: &str) -> Vec<&'static str> {
    if word.len() > 64 {
        return Vec::new();
    }
    let limit = (word.chars().count() / 3).max(2);
    let mut near: Vec<(usize, &'static str)> = CommandName::ALL
        .into_iter()
        .map(CommandName::as_str)
        .chain(["data-report"])
        .map(|name| (distance(word, name), name))
        .filter(|(d, _)| *d <= limit)
        .collect();
    near.sort();
    near.into_iter().take(3).map(|(_, name)| name).collect()
}

/// The command of one logical line.
pub fn parse_line(line: &Line) -> Result<Command, ScriptError> {
    let Some((head, rest)) = line.words.split_first() else {
        // The lexer emits no empty line; a caller's line without words is a missing command.
        return Err(ScriptError::new(
            line.pos,
            ScriptErrorCode::MissingArgument,
            "a command line needs a command",
        ));
    };
    let Some(name) = CommandName::parse(&head.text) else {
        let near = suggestions(&head.text);
        let hint = if near.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = near.iter().map(|n| format!("`{n}`")).collect();
            format!(" (did you mean {}?)", names.join(", "))
        };
        return Err(err(
            head,
            ScriptErrorCode::UnknownCommand,
            format!("unknown command `{}`{hint}", head.text),
        ));
    };
    let args = parse_args(name, head, rest)?;
    Ok(Command {
        pos: line.pos,
        end_line: line.end_line,
        name,
        args,
        words: line.words.clone(),
    })
}

/// [`parse_line`] taking the line: its words move into the command (one copy fewer of a large script).
pub fn parse_line_owned(line: Line) -> Result<Command, ScriptError> {
    let Some((head, rest)) = line.words.split_first() else {
        return parse_line(&line);
    };
    let Some(name) = CommandName::parse(&head.text) else {
        return parse_line(&line);
    };
    let args = parse_args(name, head, rest)?;
    Ok(Command {
        pos: line.pos,
        end_line: line.end_line,
        name,
        args,
        words: line.words,
    })
}

/// Refuses a keyword of another command among positional words.
fn no_keyword(word: &Word, name: CommandName) -> Result<(), ScriptError> {
    match any_keyword(word) {
        Some(kw) => Err(foreign(word, kw, name)),
        None => Ok(()),
    }
}

pub(crate) fn parse_args(
    name: CommandName,
    head: &Word,
    rest: &[Word],
) -> Result<Args<Word>, ScriptError> {
    let at_end = rest.last().unwrap_or(head);
    let exactly_one = |what: &str| -> Result<Word, ScriptError> {
        match rest {
            [] => Err(missing(head, name, what)),
            [one] => Ok(one.clone()),
            [_, extra, ..] => Err(unexpected(extra, name)),
        }
    };
    let some = |what: &str| -> Result<Vec<Word>, ScriptError> {
        if rest.is_empty() {
            Err(missing(head, name, what))
        } else {
            Ok(rest.to_vec())
        }
    };
    Ok(match name {
        CommandName::Load => Args::Load(parse_load(head, rest)?),
        CommandName::Criteria => Args::Criteria {
            words: some("at least one criterion")?,
        },
        CommandName::Filter => Args::Filter(parse_filter(&exactly_one(
            "masks, cutoff:, size:, attrib:, unixtype:, exclude-protected or include-protected",
        )?)),
        CommandName::Expand | CommandName::Collapse => {
            let words = some("`all` or the paths of folders")?;
            let args = match words.as_slice() {
                [one] if is(one, "all") => ExpandArgs::All,
                _ => ExpandArgs::Paths(words),
            };
            if name == CommandName::Expand {
                Args::Expand(args)
            } else {
                Args::Collapse(args)
            }
        }
        CommandName::Select => Args::Select {
            masks: some("at least one select mask")?,
        },
        CommandName::Compare => match rest {
            [] => Args::Compare { method: None },
            [one] => Args::Compare {
                method: Some(one.clone()),
            },
            [_, extra, ..] => return Err(unexpected(extra, name)),
        },
        CommandName::Copy | CommandName::Move => {
            let direction = exactly_one("a direction, left->right or right->left")?;
            if !direction.expands && !direction.text.contains("->") {
                return Err(invalid(
                    &direction,
                    format!(
                        "{} needs a direction, left->right or right->left, not `{}`",
                        name.as_str(),
                        direction.text
                    ),
                ));
            }
            if name == CommandName::Copy {
                Args::Copy { direction }
            } else {
                Args::Move { direction }
            }
        }
        CommandName::CopyTo | CommandName::MoveTo => {
            let args = parse_copy_to(name, head, rest)?;
            if name == CommandName::CopyTo {
                Args::CopyTo(args)
            } else {
                Args::MoveTo(args)
            }
        }
        CommandName::Delete => parse_delete(head, rest)?,
        CommandName::Rename => match rest {
            [] => {
                return Err(missing(
                    head,
                    name,
                    "a new name mask, or regexpr <old> <new>",
                ));
            }
            [first, ..] if is(first, "regexpr") => match rest {
                [_, old, new] => Args::Rename(RenameArgs::Regex {
                    old: old.clone(),
                    new: new.clone(),
                }),
                [_, _, _, extra, ..] => return Err(unexpected(extra, name)),
                _ => return Err(missing(at_end, name, "regexpr <old> <new>")),
            },
            [mask] => Args::Rename(RenameArgs::Mask(mask.clone())),
            [_, extra, ..] => return Err(unexpected(extra, name)),
        },
        CommandName::Touch => {
            let word = exactly_one("left->right, right->left, or <sides>:now or <sides>:<time>")?;
            if word.text.contains("->") {
                Args::Touch(TouchArgs::Direction(word))
            } else if let Some((sides, time)) = word.text.split_once(':') {
                let column = word.pos.column + sides.chars().count() as u32 + 1;
                Args::Touch(TouchArgs::At {
                    sides: Word::new(sides, word.quoted, word.pos),
                    time: Word::new(
                        time,
                        word.quoted,
                        crate::error::Pos::new(word.pos.line, column),
                    ),
                })
            } else if word.expands {
                // Its value decides at run time; a word without `->` or `:` needs one there.
                Args::Touch(TouchArgs::Direction(word))
            } else {
                return Err(invalid(
                    &word,
                    format!(
                        "touch needs left->right, right->left, or <sides>:now or <sides>:<time>, not `{}`",
                        word.text
                    ),
                ));
            }
        }
        CommandName::Attrib => Args::Attrib {
            changes: some("at least one change, such as +r or -h")?,
        },
        CommandName::Sync => parse_sync(rest)?,
        CommandName::Snapshot => Args::Snapshot {
            words: rest.to_vec(),
        },
        CommandName::Option => {
            let word = exactly_one("stop-on-error or confirm:<mode>")?;
            if is(&word, "stop-on-error") {
                Args::Option(OptionArgs::StopOnError)
            } else if let Some(mode) = keyword(&word, "confirm:") {
                Args::Option(OptionArgs::Confirm(mode))
            } else {
                return Err(invalid(
                    &word,
                    format!(
                        "option takes stop-on-error or confirm:prompt|yes-to-all|no-to-all, not `{}`",
                        word.text
                    ),
                ));
            }
        }
        CommandName::Log => parse_log(head, rest)?,
        CommandName::Beep => match rest {
            [] => Args::Beep,
            [extra, ..] => return Err(unexpected(extra, name)),
        },
        CommandName::TextReport
        | CommandName::FileReport
        | CommandName::FolderReport
        | CommandName::TableReport
        | CommandName::HexReport
        | CommandName::PictureReport
        | CommandName::MediaReport
        | CommandName::VersionReport
        | CommandName::RegistryReport => Args::Report(parse_report(name, head, rest)?),
    })
}

fn parse_load(head: &Word, rest: &[Word]) -> Result<LoadArgs<Word>, ScriptError> {
    let name = CommandName::Load;
    let (create, specs) = match rest.split_first() {
        Some((first, others)) => match keyword(first, "create:") {
            Some(sides) => (Some(sides), others),
            None => (None, rest),
        },
        None => (None, rest),
    };
    for spec in specs {
        no_keyword(spec, name)?;
    }
    match (create, specs) {
        (_, []) => Err(missing(
            rest.last().unwrap_or(head),
            name,
            "a session name, <default>, or two folders",
        )),
        (None, [one]) if is(one, "<default>") => Ok(LoadArgs::Default),
        (None, [one]) => Ok(LoadArgs::Session(one.clone())),
        (Some(_), [one]) => Err(missing(one, name, "two folders after create:")),
        (create, [left, right]) => Ok(LoadArgs::Folders {
            create,
            left: left.clone(),
            right: right.clone(),
        }),
        (_, [_, _, extra, ..]) => Err(unexpected(extra, name)),
    }
}

fn parse_filter(word: &Word) -> FilterArgs<Word> {
    if is(word, "exclude-protected") {
        return FilterArgs::ExcludeProtected;
    }
    if is(word, "include-protected") {
        return FilterArgs::IncludeProtected;
    }
    let value = |value: Word| (!is(&value, "none")).then_some(value);
    if let Some(v) = keyword(word, "cutoff:") {
        return FilterArgs::Cutoff(value(v));
    }
    if let Some(v) = keyword(word, "size:") {
        return FilterArgs::Size(value(v));
    }
    if let Some(v) = keyword(word, "attrib:") {
        return FilterArgs::Attrib(value(v));
    }
    if let Some(v) = keyword(word, "unixtype:") {
        return FilterArgs::UnixType(value(v));
    }
    FilterArgs::Masks(word.clone())
}

fn parse_copy_to(
    name: CommandName,
    head: &Word,
    rest: &[Word],
) -> Result<CopyToArgs<Word>, ScriptError> {
    let Some((target, before)) = rest.split_last() else {
        return Err(missing(head, name, "a target folder or archive"));
    };
    if keyword(target, "path:").is_some() {
        return Err(missing(target, name, "a target after path:"));
    }
    no_keyword(target, name)?;
    let mut sides: Option<Word> = None;
    let mut structure: Option<Word> = None;
    for word in before {
        if let Some(value) = keyword(word, "path:") {
            if structure.is_some() {
                return Err(duplicate(word, "path:"));
            }
            structure = Some(value);
        } else if word.text.contains("->") && !word.expands {
            return Err(invalid(
                word,
                format!(
                    "{} takes sides (left, right, all), not a direction `{}`",
                    name.as_str(),
                    word.text
                ),
            ));
        } else if sides.is_none() && (is_side_word(&word.text) || word.expands) {
            sides = Some(word.clone());
        } else if let Some(kw) = any_keyword(word) {
            return Err(foreign(word, kw, name));
        } else {
            return Err(unexpected(word, name));
        }
    }
    Ok(CopyToArgs {
        sides,
        structure,
        target: target.clone(),
    })
}

fn parse_delete(head: &Word, rest: &[Word]) -> Result<Args<Word>, ScriptError> {
    let name = CommandName::Delete;
    let (recycle, sides) = match rest {
        [] => return Err(missing(head, name, "the sides: left, right or all")),
        [first, others @ ..] => {
            match keyword(first, "recyclebin=").or_else(|| keyword(first, "recyclebin:")) {
                Some(value) => {
                    if !value.expands && !is(&value, "yes") && !is(&value, "no") {
                        return Err(invalid(
                            first,
                            format!("recyclebin takes yes or no, not `{}`", value.text),
                        ));
                    }
                    (Some(value), others)
                }
                None => (None, rest),
            }
        }
    };
    let sides = match sides {
        [] => {
            return Err(missing(
                rest.last().unwrap_or(head),
                name,
                "the sides: left, right or all",
            ));
        }
        [one] => one.clone(),
        [first, extra, ..] => {
            if keyword(extra, "recyclebin=").is_some() || keyword(extra, "recyclebin:").is_some() {
                return Err(if recycle.is_some() {
                    duplicate(extra, "recyclebin=")
                } else {
                    unexpected(first, name)
                });
            }
            return Err(unexpected(extra, name));
        }
    };
    if sides.text.contains("->") && !sides.expands {
        return Err(invalid(
            &sides,
            format!(
                "delete takes sides (left, right, all), not a direction `{}`",
                sides.text
            ),
        ));
    }
    Ok(Args::Delete { recycle, sides })
}

fn parse_sync(rest: &[Word]) -> Result<Args<Word>, ScriptError> {
    let name = CommandName::Sync;
    let mut args = SyncArgs {
        visible: false,
        create_empty: false,
        mode: None,
    };
    for word in rest {
        let mode = keyword(word, "update:")
            .map(|v| (SyncMode::Update, v, "update:"))
            .or_else(|| keyword(word, "mirror:").map(|v| (SyncMode::Mirror, v, "mirror:")));
        if is(word, "visible") {
            if args.visible {
                return Err(duplicate(word, "visible"));
            }
            args.visible = true;
        } else if is(word, "create-empty") {
            if args.create_empty {
                return Err(duplicate(word, "create-empty"));
            }
            args.create_empty = true;
        } else if let Some((mode, value, kw)) = mode {
            if args.mode.is_some() {
                return Err(duplicate(word, kw));
            }
            if mode == SyncMode::Mirror && !value.expands && is(&value, "all") {
                return Err(invalid(
                    word,
                    "mirror:all is not a sync: a mirror goes one way, mirror:left->right or mirror:right->left"
                        .to_owned(),
                ));
            }
            args.mode = Some((mode, value));
        } else if let Some(kw) = any_keyword(word) {
            return Err(foreign(word, kw, name));
        } else {
            return Err(unexpected(word, name));
        }
    }
    Ok(Args::Sync(args))
}

fn parse_log(head: &Word, rest: &[Word]) -> Result<Args<Word>, ScriptError> {
    let name = CommandName::Log;
    let is_level = |word: &Word| {
        ["none", "normal", "verbose"]
            .into_iter()
            .any(|l| is(word, l))
    };
    let (level, file) = match rest {
        [] => {
            return Err(missing(
                head,
                name,
                "a level (none, normal, verbose) or a file",
            ));
        }
        [one] if is_level(one) => (Some(one.clone()), None),
        [one] => (None, Some(one)),
        [level, file] if is_level(level) => (Some(level.clone()), Some(file)),
        [first, second] => {
            let _ = first;
            return Err(unexpected(second, name));
        }
        [_, _, extra, ..] => return Err(unexpected(extra, name)),
    };
    let (file, append) = match file {
        Some(word) => match keyword(word, "append:") {
            Some(value) if value.text.is_empty() => {
                return Err(missing(word, name, "a file after append:"));
            }
            Some(value) => (Some(value), true),
            None => {
                no_keyword(word, name)?;
                (Some(word.clone()), false)
            }
        },
        None => (None, false),
    };
    Ok(Args::Log(LogArgs {
        level,
        file,
        append,
    }))
}

fn parse_report(
    name: CommandName,
    head: &Word,
    rest: &[Word],
) -> Result<ReportArgs<Word>, ScriptError> {
    let mut args = ReportArgs {
        just_selection: false,
        layout: None,
        options: Vec::new(),
        output_options: Vec::new(),
        title: None,
        output_to: None,
        specs: Vec::new(),
    };
    let single = |slot: &mut Option<Word>, word: &Word, value: Word, kw: &str| {
        if slot.is_some() {
            return Err(duplicate(word, kw));
        }
        *slot = Some(value);
        Ok(())
    };
    for word in rest {
        if name == CommandName::FolderReport && is(word, "just-selection") {
            if args.just_selection {
                return Err(duplicate(word, "just-selection"));
            }
            args.just_selection = true;
        } else if let Some(value) = keyword(word, "layout:") {
            single(&mut args.layout, word, value, "layout:")?;
        } else if let Some(value) = keyword(word, "options:") {
            args.options.push(value);
        } else if let Some(value) = keyword(word, "output-options:") {
            args.output_options.push(value);
        } else if let Some(value) = keyword(word, "title:") {
            single(&mut args.title, word, value, "title:")?;
        } else if let Some(value) = keyword(word, "output-to:") {
            if value.text.is_empty() {
                return Err(missing(word, name, "a target after output-to:"));
            }
            single(&mut args.output_to, word, value, "output-to:")?;
        } else if let Some(kw) = any_keyword(word) {
            return Err(foreign(word, kw, name));
        } else {
            args.specs.push(word.clone());
        }
    }
    let at_end = rest.last().unwrap_or(head);
    match name.report() {
        Some(ReportFor::Folder) => {
            if let Some(extra) = args.specs.first() {
                return Err(unexpected(extra, name));
            }
        }
        Some(ReportFor::Text) => {
            if let Some(extra) = args.specs.get(2) {
                return Err(unexpected(extra, name));
            }
            if args.specs.is_empty() && name == CommandName::TextReport {
                return Err(missing(at_end, name, "a text-compare session or two files"));
            }
        }
        Some(ReportFor::Other) | None => {
            if let Some(extra) = args.specs.get(2) {
                return Err(unexpected(extra, name));
            }
        }
    }
    if args.output_to.is_none() && name.report() != Some(ReportFor::Other) {
        return Err(missing(
            at_end,
            name,
            "output-to: (a file, clipboard, printer, browser, or - for standard output)",
        ));
    }
    Ok(args)
}
