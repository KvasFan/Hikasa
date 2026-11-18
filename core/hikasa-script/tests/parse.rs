//! A8.2: one fixture per grammar form of §1.8.6 and the four examples of docs/12 §3.2, each with its golden in the
//! `commands` form of `script check --json` (`line`, `end_line`, `name`, `words`).

mod common;

use hikasa_script::{AnyWord, Args, CopyToArgs, LoadArgs, Script, parse};
use serde::Serialize;

#[derive(Serialize)]
struct Shown<'a> {
    line: u32,
    end_line: u32,
    name: &'static str,
    words: Vec<String>,
    args: &'a Args<hikasa_script::Word>,
}

fn render(script: &Script) -> String {
    let made: Vec<hikasa_script::Command> = script.commands().collect();
    assert_eq!(made.len(), script.len());
    let commands: Vec<Shown<'_>> = made
        .iter()
        .map(|c| Shown {
            line: c.pos.line,
            end_line: c.end_line,
            name: c.name.as_str(),
            words: c.word_texts(),
            args: &c.args,
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&serde_json::json!({
        "commands": commands,
        "references": script.references,
    }))
    .unwrap();
    text.push('\n');
    text
}

#[test]
fn goldens() {
    let scripts = common::scripts("parse");
    assert!(scripts.len() >= 28, "{}", scripts.len());
    for script in scripts {
        let source = std::fs::read(&script).unwrap();
        let parsed = parse(&source, &AnyWord)
            .unwrap_or_else(|e| panic!("{}: {:?}", script.display(), e.lines("s")));
        common::golden(&script.with_extension("json"), &render(&parsed));
    }
}

#[test]
fn copyto_left_alone_is_a_target() {
    let script = parse(b"load a b\ncopyto left\n", &AnyWord).unwrap();
    let command = script.command(1).unwrap();
    let Args::CopyTo(CopyToArgs { sides, target, .. }) = &command.args else {
        panic!("{:?}", command.args);
    };
    assert!(sides.is_none());
    assert_eq!(target.text, "left");
}

#[test]
fn default_in_any_case() {
    for word in ["<default>", "<DEFAULT>", "<Default>"] {
        let script = parse(format!("load {word}").as_bytes(), &AnyWord).unwrap();
        assert_eq!(
            script.command(0).unwrap().args,
            Args::Load(LoadArgs::Default)
        );
    }
}
