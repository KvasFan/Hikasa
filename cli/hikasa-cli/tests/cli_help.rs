//! `hikasa help` (M2 W0; docs/12 §1): topics as text and `hikasa.help.v1`, `--list`, unknown topics (exit 100
//! with suggestions), and one topic for every subcommand of the command tree.

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

#[test]
fn the_contents_and_a_topic() {
    let s = Sandbox::new();
    let index = s.cmd().arg("help").output().unwrap();
    assert_eq!(code(&index), 0);
    assert!(stdout(&index).starts_with("Hikasa Help\n===========\n"));
    let topic = s.cmd().args(["help", "text", "compare"]).output().unwrap();
    assert_eq!(code(&topic), 0);
    assert_golden("help/text-compare.txt", &stdout(&topic));
    let by_id = s.cmd().args(["help", "text-compare"]).output().unwrap();
    assert_eq!(stdout(&by_id), stdout(&topic));
    // A subcommand without its own topic shows its parent's.
    let parent = s.cmd().args(["help", "format", "list"]).output().unwrap();
    assert!(stdout(&parent).starts_with("hikasa format\n"));
}

#[test]
fn json_and_list() {
    let s = Sandbox::new();
    let out = s
        .cmd()
        .args(["help", "exit-codes", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.help.v1");
    assert_eq!(doc["result"]["topic"]["id"], "exit-codes");
    assert_eq!(doc["result"]["topics"], serde_json::json!([]));
    assert_golden("help/exit-codes.json", &s.normalise(&stdout(&out)));
    let list = s.cmd().args(["help", "--list"]).output().unwrap();
    assert_eq!(code(&list), 0);
    assert_golden("help/list.txt", &stdout(&list));
    let list_json = s.cmd().args(["--json", "help", "--list"]).output().unwrap();
    let doc = json(&list_json);
    assert_eq!(doc["result"]["topic"], serde_json::Value::Null);
    assert_eq!(
        doc["result"]["topics"].as_array().map(Vec::len),
        Some(stdout(&list).lines().count())
    );
}

#[test]
fn unknown_topics_are_100_with_suggestions() {
    let s = Sandbox::new();
    let out = s
        .cmd()
        .args(["help", "fodler", "compare"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(out.stdout.is_empty());
    assert!(
        stderr(&out).contains("did you mean: folder-compare"),
        "{}",
        stderr(&out)
    );
    let doc = json(
        &s.cmd()
            .args(["help", "nothing-like-this", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(doc["schema"], "hikasa.help.v1");
    assert_eq!(doc["exit_code"], 100);
    assert_eq!(doc["error"]["kind"], "invalid_input");
    let both = s.cmd().args(["help", "--list", "quick"]).output().unwrap();
    assert_eq!(code(&both), 100, "--list takes no topic");
}

/// The definition of done: every subcommand of the command tree has a help topic (its own or its parent's).
#[test]
fn every_subcommand_has_a_topic() {
    let s = Sandbox::new();
    let tree = [
        "quick",
        "text compare",
        "folder compare",
        "format",
        "vfs",
        "gui",
        "completions",
        "version",
        "help",
    ];
    for words in tree {
        let mut args = vec!["help"];
        args.extend(words.split(' '));
        let out = s.cmd().args(&args).output().unwrap();
        assert_eq!(code(&out), 0, "{words}");
        let first = stdout(&out).lines().next().unwrap_or_default().to_owned();
        assert_eq!(first, format!("hikasa {words}"), "{words}");
    }
}

/// `help` reads no configuration: it works with a configuration that does not load.
#[test]
fn help_needs_no_configuration() {
    let s = Sandbox::new();
    s.file("cfg/options.toml", "this is = = not toml");
    let out = s.cmd().args(["help", "quick"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}
