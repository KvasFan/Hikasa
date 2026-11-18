//! P-LEX, P-PARSE, P-EXPAND (A8.1, A8.2, A8.4): the lexer is total and its error positions lie inside the input; printed
//! words and commands lex and parse back to themselves; escaping a literal's `%` and expanding gives the literal.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use hikasa_script::{AnyWord, Args, ExpandContext, LocalTime, Pos, Word, expand, lex, parse};
use proptest::prelude::*;

/// Characters a word may hold: blanks, quotes, `#`, `&`, `%`, NBSP, letters, backslashes (no line end).
fn word_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just(' '),
            Just('\t'),
            Just('"'),
            Just('#'),
            Just('&'),
            Just('%'),
            Just('\u{a0}'),
            Just('\\'),
            Just(':'),
            Just('é'),
            proptest::char::range('a', 'e'),
            proptest::char::range('0', '3'),
        ],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// A word to print: its text, and whether it was quoted.
fn word() -> impl Strategy<Value = Word> {
    (word_text(), any::<bool>()).prop_map(|(text, quoted)| Word::new(text, quoted, Pos::START))
}

/// The line and the column count of each physical line of `text` (LF, CRLF, CR).
fn line_lengths(text: &str) -> Vec<u32> {
    let mut out = vec![0u32];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(0);
            }
            '\n' => out.push(0),
            _ => *out.last_mut().unwrap() += 1,
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256)
    ))]

    /// P-LEX: any bytes lex without a panic, and every error position lies inside the input.
    #[test]
    fn p_lex_total(bytes in prop_oneof![
        proptest::collection::vec(
            prop_oneof![
                Just(b' '), Just(b'"'), Just(b'#'), Just(b'&'), Just(b'%'), Just(b'\n'), Just(b'\r'),
                Just(b'\t'), Just(0xff), Just(0xc3), Just(0xa9), Just(b'a'), Just(b'1'),
            ],
            0..64,
        ),
        "[ \"#&%\n\r\ta1é\u{a0}]{0,40}".prop_map(String::into_bytes),
    ]) {
        let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        match lex(&bytes) {
            Ok(lines) => {
                let text = std::str::from_utf8(body).unwrap();
                let lengths = line_lengths(text);
                let physical: Vec<Vec<char>> = text
                    .split("\r\n")
                    .flat_map(|part| part.split(['\r', '\n']))
                    .map(|line| line.chars().collect())
                    .collect();
                for line in &lines {
                    for word in &line.words {
                        let len = lengths[(word.pos.line - 1) as usize];
                        prop_assert!(word.pos.column >= 1 && word.pos.column <= len, "{word:?}");
                        // The column names the word's first character, or its opening quote.
                        let at = physical[(word.pos.line - 1) as usize][(word.pos.column - 1) as usize];
                        prop_assert!(at == '"' || word.text.starts_with(at), "{word:?} at {at:?}");
                    }
                }
            }
            Err(errors) => {
                let valid = match std::str::from_utf8(body) {
                    Ok(text) => text,
                    Err(e) => std::str::from_utf8(&body[..e.valid_up_to()]).unwrap(),
                };
                let lengths = line_lengths(valid);
                for error in &errors.list {
                    let line = error.pos.line as usize;
                    prop_assert!(line >= 1 && line <= lengths.len(), "{error:?}");
                    prop_assert!(error.pos.column >= 1 && error.pos.column <= lengths[line - 1] + 1, "{error:?}");
                }
            }
        }
    }

    /// P-LEX: words printed with `Word::to_source` lex back to the same words.
    #[test]
    fn p_lex_round_trip(words in proptest::collection::vec(word(), 1..6)) {
        let mut all = vec![Word::new("load", false, Pos::START)];
        all.extend(words);
        let line: Vec<String> = all.iter().map(Word::to_source).collect();
        let source = line.join(" ");
        let lines = lex(source.as_bytes()).unwrap();
        prop_assert_eq!(lines.len(), 1, "{}", source);
        let texts: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        let wanted: Vec<&str> = all.iter().map(|w| w.text.as_str()).collect();
        prop_assert_eq!(texts, wanted, "{}", source);
        for (got, want) in lines[0].words.iter().zip(&all) {
            if want.quoted {
                prop_assert!(got.quoted);
            }
            prop_assert_eq!(got.expands, want.expands);
        }
    }

    /// P-PARSE: random commands from the grammar, printed and parsed again, are equal.
    #[test]
    fn p_parse_round_trip(commands in proptest::collection::vec(command(), 1..8)) {
        let mut source = String::from("load \"a b\" c\n");
        for command in &commands {
            source.push_str(command);
            source.push('\n');
        }
        let first = parse(source.as_bytes(), &AnyWord).map_err(|e| TestCaseError::fail(format!("{:?}\n{source}", e.lines("s"))))?;
        let printed: Vec<String> = first.commands().map(|c| c.source()).collect();
        let again = parse(printed.join("\n").as_bytes(), &AnyWord).unwrap();
        prop_assert_eq!(first.len(), again.len());
        prop_assert_eq!(first.commands().count(), first.len());
        for (a, b) in first.commands().zip(again.commands()) {
            prop_assert_eq!(a.name, b.name);
            prop_assert_eq!(texts(&a.args), texts(&b.args));
            prop_assert_eq!(a.word_texts(), b.word_texts());
        }
    }

    /// P-EXPAND: escaping a literal's `%` as `%%` and expanding gives the literal.
    #[test]
    fn p_expand_escaped_literal(literal in word_text()) {
        let escaped = literal.replace('%', "%%");
        let env = BTreeMap::new();
        let ctx = ExpandContext { script: OsStr::new("s"), args: &[], env: &env, env_case_insensitive: false, now: LocalTime::default() };
        prop_assert_eq!(expand(&Word::new(escaped, false, Pos::START), &ctx).unwrap(), OsString::from(literal));
    }

    /// P-EXPAND: expansion of random words never panics.
    #[test]
    fn p_expand_total(text in "[%a-z0-9_ ]{0,12}", args in proptest::collection::vec("[a-z%]{0,3}", 0..3)) {
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        let mut env = BTreeMap::new();
        env.insert("a".to_owned(), OsString::from("A"));
        let ctx = ExpandContext { script: OsStr::new("s"), args: &args, env: &env, env_case_insensitive: true, now: LocalTime::default() };
        let _ = expand(&Word::new(text, false, Pos::START), &ctx);
    }
}

/// The texts of the words of `args`.
fn texts(args: &Args<Word>) -> Args<String> {
    args.try_map(&mut |_, w: &Word| -> Result<String, ()> { Ok(w.text.clone()) })
        .unwrap()
}

/// A path-like word: never a keyword, never empty after quoting.
fn path() -> impl Strategy<Value = String> {
    "[a-z/ #&%.é\"]{1,8}".prop_map(|p| Word::new(format!("x{p}"), false, Pos::START).to_source())
}

fn side() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("left"),
        Just("right"),
        Just("all"),
        Just("lt"),
        Just("RT")
    ]
}

fn direction() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("left->right"), Just("right->left"), Just("lt->rt")]
}

/// One command of the grammar of §1.8.6, as a script writes it.
fn command() -> impl Strategy<Value = String> {
    prop_oneof![
        (path(), path()).prop_map(|(l, r)| format!("load {l} {r}")),
        (side(), path(), path()).prop_map(|(s, l, r)| format!("load create:{s} {l} {r}")),
        path().prop_map(|s| format!("load {s}")),
        Just("load <Default>\nload a b".to_owned()),
        Just("criteria crc size timestamp:2sec".to_owned()),
        path().prop_map(|m| format!("filter {m}")),
        Just("filter cutoff:none".to_owned()),
        Just("filter size:>1KB".to_owned()),
        proptest::collection::vec(path(), 1..3).prop_map(|p| format!("expand {}", p.join(" "))),
        Just("collapse all".to_owned()),
        Just("select left.diff.files lt.orphan all".to_owned()),
        prop_oneof![Just(""), Just(" crc"), Just(" binary")].prop_map(|m| format!("compare{m}")),
        direction().prop_map(|d| format!("copy {d}")),
        direction().prop_map(|d| format!("move {d}")),
        (
            proptest::option::of(side()),
            proptest::option::of(prop_oneof![Just("base"), Just("none")]),
            path()
        )
            .prop_map(|(s, p, t)| format!(
                "copyto {}{}{t}",
                s.map_or(String::new(), |s| format!("{s} ")),
                p.map_or(String::new(), |p| format!("path:{p} "))
            )),
        (side(), path()).prop_map(|(s, t)| format!("moveto {s} {t}")),
        (
            proptest::option::of(prop_oneof![Just("yes"), Just("no")]),
            side()
        )
            .prop_map(|(r, s)| format!(
                "delete {}{s}",
                r.map_or(String::new(), |r| format!("recyclebin={r} "))
            )),
        path().prop_map(|m| format!("rename {m}")),
        (path(), path()).prop_map(|(o, n)| format!("rename regexpr {o} {n}")),
        direction().prop_map(|d| format!("touch {d}")),
        side().prop_map(|s| format!("touch {s}:now")),
        Just("attrib +r -h".to_owned()),
        (
            any::<bool>(),
            any::<bool>(),
            proptest::option::of(direction())
        )
            .prop_map(|(v, c, d)| format!(
                "sync{}{}{}",
                if v { " visible" } else { "" },
                if c { " create-empty" } else { "" },
                d.map_or(String::new(), |d| format!(" update:{d}"))
            )),
        (path(), path(), path())
            .prop_map(|(o, l, r)| format!("text-report layout:summary output-to:{o} {l} {r}")),
        path().prop_map(|o| format!(
            "folder-report just-selection options:display-all output-to:{o}"
        )),
        Just("option stop-on-error".to_owned()),
        Just("option confirm:yes-to-all".to_owned()),
        path().prop_map(|f| format!("log verbose append:{f}")),
        Just("beep".to_owned()),
        Just("snapshot save-crc left".to_owned()),
    ]
}
