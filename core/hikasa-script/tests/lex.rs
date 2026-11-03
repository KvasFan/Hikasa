//! A8.1: the lexer's goldens (`tests/fixtures/lex/<name>.txt` → `<name>.json`) and the size limit.

mod common;

use hikasa_script::{Errors, Line, SCRIPT_MAX_BYTES, ScriptErrorCode, lex};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Lexed<'a> {
    Lines(&'a [Line]),
    Errors(&'a Errors),
}

fn render(source: &[u8]) -> String {
    let mut text = match lex(source) {
        Ok(lines) => serde_json::to_string_pretty(&Lexed::Lines(&lines)).unwrap(),
        Err(errors) => serde_json::to_string_pretty(&Lexed::Errors(&errors)).unwrap(),
    };
    text.push('\n');
    text
}

#[test]
fn goldens() {
    let scripts = common::scripts("lex");
    assert!(scripts.len() >= 12, "{scripts:?}");
    for script in scripts {
        let source = std::fs::read(&script).unwrap();
        common::golden(&script.with_extension("json"), &render(&source));
    }
}

#[test]
fn sixteen_mib_exactly_is_read_and_one_more_byte_is_too_large() {
    let max = usize::try_from(SCRIPT_MAX_BYTES).unwrap();
    let mut source = vec![b' '; max];
    source[..5].copy_from_slice(b"beep\n");
    let lines = lex(&source).unwrap();
    assert_eq!(lines.len(), 1);
    source.push(b' ');
    let errors = lex(&source).unwrap_err();
    assert_eq!(errors.list.len(), 1);
    assert_eq!(errors.list[0].code, ScriptErrorCode::TooLarge);
    assert!(errors.is_load());
}

#[test]
fn an_empty_quoted_word_is_a_word() {
    let lines = lex(b"select \"\"").unwrap();
    assert_eq!(lines[0].words.len(), 2);
    assert_eq!(lines[0].words[1].text, "");
    assert!(lines[0].words[1].quoted);
}

#[test]
fn a_quoted_ampersand_does_not_continue() {
    let lines = lex(b"load \"x&\" y\nbeep\n").unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].words[1].text, "x&");
    let lines = lex(b"load x\"&\"\nbeep\n").unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].words[1].text, "x&");
}

#[test]
fn split_words_follows_the_word_rules() {
    assert_eq!(
        hikasa_script::split_words("a \"b c\" d\"\"e x&").unwrap(),
        vec!["a", "b c", "de", "x&"]
    );
    assert_eq!(
        hikasa_script::split_words("\"open").unwrap_err().code,
        ScriptErrorCode::UnterminatedQuote
    );
}
