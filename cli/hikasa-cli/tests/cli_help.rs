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

/// The subcommands `hikasa <words…> --help` lists under "Commands:" (clap's own `help` left out).
fn subcommands_of(s: &Sandbox, words: &[String]) -> Vec<String> {
    let out = s.cmd().args(words).arg("--help").output().unwrap();
    assert_eq!(code(&out), 0, "{words:?} --help");
    let text = stdout(&out);
    let mut names = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line == "Commands:" {
            inside = true;
            continue;
        }
        if inside {
            if line.trim().is_empty() {
                break;
            }
            // A command line starts with two spaces and its name; a wrapped description with more spaces.
            if let Some(rest) = line.strip_prefix("  ")
                && !rest.starts_with(' ')
                && let Some(name) = rest.split_whitespace().next()
                && name != "help"
            {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// The definition of done (M2 W14, the release's help coverage): every subcommand of the command tree, walked
/// from clap's own `--help`, has a help topic — its own or its parent's (`format list` shows `hikasa format`) —
/// and the commands M2 added have their own.
#[test]
fn every_subcommand_has_a_topic() {
    let s = Sandbox::new();
    let mut pending: Vec<Vec<String>> = vec![Vec::new()];
    let mut leaves = 0;
    while let Some(words) = pending.pop() {
        let children = subcommands_of(&s, &words);
        if !words.is_empty() && children.is_empty() {
            leaves += 1;
            let mut args = vec!["help".to_owned()];
            args.extend(words.iter().cloned());
            let out = s.cmd().args(&args).output().unwrap();
            assert_eq!(code(&out), 0, "{words:?}: {}", stderr(&out));
            let first = stdout(&out).lines().next().unwrap_or_default().to_owned();
            let own_or_parent =
                (1..=words.len()).any(|n| first == format!("hikasa {}", words[..n].join(" ")));
            assert!(own_or_parent, "{words:?} shows {first:?}");
        }
        for child in children {
            let mut next = words.clone();
            next.push(child);
            pending.push(next);
        }
    }
    assert!(leaves >= 60, "the walk found only {leaves} subcommands");
    // Their own topics, not a parent's.
    for words in [
        "quick",
        "text compare",
        "text convert",
        "text find",
        "table compare",
        "hex compare",
        "hex dump",
        "hex find",
        "picture compare",
        "folder compare",
        "folder op",
        "folder sync",
        "report",
        "script",
        "format",
        "settings",
        "session",
        "workspace",
        "vfs",
        "gui",
        "completions",
        "version",
        "help",
    ] {
        let mut args = vec!["help"];
        args.extend(words.split(' '));
        let out = s.cmd().args(&args).output().unwrap();
        assert_eq!(code(&out), 0, "{words}");
        let first = stdout(&out).lines().next().unwrap_or_default().to_owned();
        assert_eq!(first, format!("hikasa {words}"), "{words}");
    }
    // CLI-010's manual topic.
    let vcs = s.cmd().args(["help", "version-control"]).output().unwrap();
    assert_eq!(code(&vcs), 0);
    assert!(stdout(&vcs).starts_with("Using Hikasa with version control\n"));
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
