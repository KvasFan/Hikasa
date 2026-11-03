//! A8.4 (pure part): the forms of §1.8.4 row by row, one instant per command, values never re-lexed, `lt`/`rt` only in
//! side positions (A8.5's table), and the bytes of a non-UTF-8 argument.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use hikasa_script::{
    AnyWord, ExpandContext, LocalTime, Pos, ScriptErrorCode, Slot, Word, expand, normalize_sides,
    parse,
};

const NOW: LocalTime = LocalTime {
    year: 2026,
    month: 3,
    day: 8,
    hour: 7,
    minute: 5,
    second: 9,
};

fn ctx<'a>(
    args: &'a [OsString],
    env: &'a BTreeMap<String, OsString>,
    ci: bool,
) -> ExpandContext<'a> {
    ExpandContext {
        script: OsStr::new("/s/run.txt"),
        args,
        env,
        env_case_insensitive: ci,
        now: NOW,
    }
}

fn word(text: &str) -> Word {
    Word::new(text, false, Pos::START)
}

#[test]
fn the_form_table() {
    let args: Vec<OsString> = ["a", "b c", "\"q\""]
        .into_iter()
        .map(OsString::from)
        .collect();
    let mut env = BTreeMap::new();
    env.insert("HOME_X".to_owned(), OsString::from("/h"));
    env.insert("date".to_owned(), OsString::from("not this"));
    let c = ctx(&args, &env, false);
    let table = [
        ("%1", "a"),
        ("%2", "b c"),
        ("%3", "\"q\""),
        ("%10", "a0"),
        ("%0", "/s/run.txt"),
        ("%%", "%"),
        ("%%1", "%1"),
        ("50%", "50%"),
        ("%x y", "%x y"),
        ("%-", "%-"),
        ("a%", "a%"),
        ("%date%", "2026-03-08"),
        ("%DATE%", "2026-03-08"),
        ("%Time%", "07:05:09"),
        ("%fn_time%", "07-05-09"),
        ("%FN_TIME%.zip", "07-05-09.zip"),
        ("%HOME_X%/x", "/h/x"),
        ("x%HOME_X", "x%HOME_X"),
        ("%1%2", "ab c"),
    ];
    for (text, expected) in table {
        assert_eq!(
            expand(&word(text), &c).unwrap(),
            OsString::from(expected),
            "{text}"
        );
    }
}

#[test]
fn environment_case() {
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), OsString::from("/bin"));
    let args = [];
    assert_eq!(
        expand(&word("%Path%"), &ctx(&args, &env, true)).unwrap(),
        OsString::from("/bin")
    );
    let error = expand(&word("%Path%"), &ctx(&args, &env, false)).unwrap_err();
    assert_eq!(error.code, ScriptErrorCode::UnsetVariable);
    assert_eq!(
        expand(&word("%1"), &ctx(&args, &env, false))
            .unwrap_err()
            .code,
        ScriptErrorCode::MissingScriptArgument
    );
}

#[test]
fn a_value_with_spaces_and_quotes_stays_one_word() {
    let script = parse(b"load %1 %2\n", &AnyWord).unwrap();
    assert_eq!(script.commands[0].words.len(), 3);
    let args: Vec<OsString> = vec!["a \"b\" & # c".into(), "%2".into()];
    let env = BTreeMap::new();
    let c = ctx(&args, &env, false);
    let words: Vec<OsString> = script.commands[0].words[1..]
        .iter()
        .map(|w| expand(w, &c).unwrap())
        .collect();
    // Never lexed again: the quotes, `&`, `#` and a `%` in a value are as given.
    assert_eq!(
        words,
        vec![OsString::from("a \"b\" & # c"), OsString::from("%2")]
    );
}

#[test]
fn set_but_empty_expands_to_nothing() {
    let mut env = BTreeMap::new();
    env.insert("E".to_owned(), OsString::new());
    assert_eq!(
        expand(&word("[%E%]"), &ctx(&[], &env, false)).unwrap(),
        OsString::from("[]")
    );
}

#[cfg(unix)]
#[test]
fn a_non_utf8_argument_keeps_its_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let args = vec![OsStr::from_bytes(b"/t/\xff\xfe").to_owned()];
    let env = BTreeMap::new();
    let value = expand(&word("%1/x"), &ctx(&args, &env, false)).unwrap();
    assert_eq!(value.as_bytes(), b"/t/\xff\xfe/x");
    assert_eq!(hikasa_script::display_os(&value), "/t/\\xFF\\xFE/x");
}

#[test]
fn lt_and_rt_only_in_side_positions() {
    let yes = [
        (Slot::Sides, "lt", "left"),
        (Slot::Sides, "RT", "right"),
        (Slot::Side, "Lt", "left"),
        (Slot::Direction, "lt->rt", "left->right"),
        (Slot::Direction, "RT->left", "right->left"),
        (Slot::SyncDirection, "rt->Lt", "right->left"),
        (Slot::SelectMask, "lt.diff.files", "left.diff.files"),
        (Slot::SelectMask, "RT", "right"),
        (Slot::Criteria, "timezone:lt+2", "timezone:left+2"),
        (Slot::Criteria, "crc,TIMEZONE:RT-3", "crc,TIMEZONE:right-3"),
    ];
    for (slot, word, expected) in yes {
        assert_eq!(
            normalize_sides(slot, word, false),
            expected,
            "{slot:?} {word}"
        );
    }
    let no = [
        (Slot::Spec, "lt"),
        (Slot::Path, "lt"),
        (Slot::NameFilter, "lt.txt"),
        (Slot::Text, "lt->rt"),
        (Slot::Session, "rt"),
        (Slot::SelectMask, "left.lt"),
        (Slot::Criteria, "xtimezone:lt+2"),
        (Slot::ReportTarget, "rt"),
    ];
    for (slot, word) in no {
        assert_eq!(normalize_sides(slot, word, false), word, "{slot:?} {word}");
    }
    // A quoted word is never rewritten.
    assert_eq!(normalize_sides(Slot::Sides, "lt", true), "lt");
}
