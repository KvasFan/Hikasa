//! `hikasa settings get|set|unset|list|restore-defaults` end to end (M2 W1, contract §6.1 A1.4, A1.10; D-71, D-73,
//! D-77): goldens in text and JSON in a temporary `--config-dir`, exit codes 0 and 100, strict loading for every
//! other command and lenient repair by `settings set`.

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// Runs `hikasa <args>` and returns (code, stdout, stderr).
fn run(s: &Sandbox, args: &[&str]) -> (i32, String, String) {
    let out = s.cmd().args(args).output().unwrap();
    (code(&out), stdout(&out), stderr(&out))
}

#[test]
fn get_set_unset_goldens() {
    let s = Sandbox::new();
    let (c, text, err) = run(
        &s,
        &[
            "settings",
            "get",
            "options.text_editing.number_of_context_lines",
        ],
    );
    assert_eq!((c, text.as_str()), (0, "3\n"), "{err}");
    let (c, text, _) = run(&s, &["settings", "get", "options.next_difference"]);
    assert_eq!(c, 0);
    assert_golden("settings/get-table.txt", &s.normalise(&text));
    let (c, text, _) = run(
        &s,
        &["settings", "get", "options.appearance.theme_mode", "--json"],
    );
    assert_eq!(c, 0);
    assert_golden("settings/get.json", &s.normalise(&text));
    assert!(
        !s.config().exists(),
        "read-only commands create nothing (D-21)"
    );

    let (c, text, err) = run(
        &s,
        &["settings", "set", "options.appearance.theme_mode", "dark"],
    );
    assert_eq!(c, 0, "{err}");
    assert_golden("settings/set.txt", &s.normalise(&text));
    assert_eq!(
        std::fs::read_to_string(s.config().join("options.toml")).unwrap(),
        "schema_version = 1\n\n[appearance]\ntheme_mode = \"dark\"\n"
    );
    let (c, text, _) = run(
        &s,
        &[
            "settings",
            "set",
            "options.appearance.theme_mode",
            "dark",
            "--json",
        ],
    );
    assert_eq!(c, 0);
    assert_golden("settings/set-unchanged.json", &s.normalise(&text));
    let (c, text, _) = run(&s, &["settings", "set", "state.window.x", "-1"]);
    assert_eq!(
        (c, text.as_str()),
        (0, "state.window.x = -1\n(file unchanged)\n")
    );
    assert!(
        !s.config().join("state.toml").exists(),
        "a default creates no file"
    );

    let (c, text, _) = run(
        &s,
        &[
            "settings",
            "unset",
            "options.appearance.theme_mode",
            "--json",
        ],
    );
    assert_eq!(c, 0);
    assert_golden("settings/unset.json", &s.normalise(&text));
    let (c, text, _) = run(&s, &["settings", "unset", "options.appearance.theme_mode"]);
    assert_eq!(
        (c, text.as_str()),
        (
            0,
            "options.appearance.theme_mode = \"follow_os\"\n(file unchanged)\n"
        )
    );
    run(
        &s,
        &[
            "settings",
            "set",
            "options.tweaks.metrics.max_mru_items",
            "9",
        ],
    );
    let (c, text, err) = run(
        &s,
        &["settings", "unset", "options.tweaks.metrics.max_mru_items"],
    );
    assert_eq!(c, 0, "{err}");
    assert_golden("settings/unset.txt", &s.normalise(&text));

    // `--force` writes over a file whatever its stamp (D-74); the result is the same as without it.
    let (c, text, err) = run(
        &s,
        &[
            "settings",
            "set",
            "options.appearance.theme_mode",
            "light",
            "--force",
            "--json",
        ],
    );
    assert_eq!(c, 0, "{err}");
    assert_golden("settings/set-force.json", &s.normalise(&text));
    assert_eq!(
        std::fs::read_to_string(s.config().join("options.toml")).unwrap(),
        "schema_version = 1\n\n[appearance]\ntheme_mode = \"light\"\n"
    );
}

#[test]
fn bad_keys_and_values_exit_100() {
    let s = Sandbox::new();
    let (c, _, err) = run(
        &s,
        &["settings", "get", "options.tweaks.metrcs.max_mru_items"],
    );
    assert_eq!(c, 100);
    assert!(
        err.contains("did you mean: options.tweaks.metrics.max_mru_items?"),
        "{err}"
    );
    let out = s
        .cmd()
        .args(["settings", "get", "options.nope", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.settings_get.v1");
    assert_eq!(doc["error"]["kind"], "not_found");
    for (args, kind) in [
        (
            vec![
                "settings",
                "set",
                "options.tweaks.metrics.max_mru_items",
                "101",
            ],
            "invalid_input",
        ),
        (
            vec![
                "settings",
                "set",
                "options.tweaks.metrics.max_mru_items",
                "x",
            ],
            "invalid_input",
        ),
        (
            vec!["settings", "set", "options.appearance.theme_mode", "pink"],
            "invalid_input",
        ),
        (
            vec!["settings", "set", "options.tweaks", "1"],
            "invalid_input",
        ),
        (
            vec!["settings", "set", "theme.dark_scheme", "nowhere"],
            "invalid_input",
        ),
        (
            vec![
                "settings",
                "set",
                "commands.view.home.command.NoSuch.menu",
                "false",
            ],
            "not_found",
        ),
        // A misspelt key under a table whose keys share one kind rule (`tabs.*`) is not a key (D-73).
        (
            vec![
                "settings",
                "set",
                "options.tabs.hide_tab_bar_if_singel_tab",
                "true",
            ],
            "not_found",
        ),
        (
            vec!["settings", "get", "options.tweaks.interface.nope"],
            "not_found",
        ),
        // An array position past the end, a command id the view does not have, a view that does not exist.
        (
            vec!["settings", "get", "openwith.app.5.command"],
            "not_found",
        ),
        (
            vec!["settings", "set", "openwith.app.0.command", "x %f"],
            "not_found",
        ),
        (
            vec![
                "settings",
                "get",
                "commands.view.home.command.Bogus.shortcuts",
            ],
            "not_found",
        ),
        (
            vec![
                "settings",
                "get",
                "commands.view.nowhere.display_filter_mode",
            ],
            "not_found",
        ),
        (
            vec![
                "settings",
                "set",
                "commands.view.home.command.Exit.shortcuts",
                "[\"Hyper+X\"]",
            ],
            "invalid_input",
        ),
        (
            vec![
                "settings",
                "set",
                "openwith.app",
                "[{ description = \"x\", command = \"x %q\" }]",
            ],
            "invalid_input",
        ),
        (vec!["settings", "get", "bogus.key"], "invalid_input"),
    ] {
        let mut args = args.clone();
        args.push("--json");
        let out = s.cmd().args(&args).output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        assert_eq!(json(&out)["error"]["kind"], kind, "{args:?}");
    }
    assert!(!s.config().exists(), "nothing written");
    let (c, _, err) = run(
        &s,
        &[
            "settings",
            "set",
            "options.tabs.hide_tab_bar_if_singel_tab",
            "true",
        ],
    );
    assert_eq!(c, 100);
    assert!(
        err.contains("did you mean: options.tabs.hide_tab_bar_if_single_tab?"),
        "{err}"
    );
}

#[test]
fn list_goldens_and_conflicts() {
    let s = Sandbox::new();
    s.file(
        "cfg/options.toml",
        "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\nlater = 1\n[colours]\nred = 1\n",
    );
    s.file(
        "cfg/commands.toml",
        "[view.home.command.Exit]\nshortcuts = [\"Ctrl+T\"]\n",
    );
    let (c, text, err) = run(&s, &["settings", "list", "--changed"]);
    assert_eq!(c, 0, "{err}");
    assert_golden("settings/list-changed.txt", &s.normalise(&text));
    assert!(
        err.contains("warning: shortcut conflict in home: Ctrl+T is bound to TabNew, Exit"),
        "{err}"
    );
    assert!(err.contains("unknown section `colours`"), "{err}");
    let (c, text, _) = run(&s, &["settings", "list", "--changed", "--json"]);
    assert_eq!(c, 0);
    assert_golden("settings/list-changed.json", &s.normalise(&text));
    let (c, text, _) = run(&s, &["settings", "list", "options.tweaks.metrics"]);
    assert_eq!(c, 0);
    assert_golden("settings/list-metrics.txt", &s.normalise(&text));
    let (c, text, _) = run(&s, &["settings", "list"]);
    assert_eq!(c, 0);
    for file in [
        "options.",
        "state.",
        "commands.",
        "theme.",
        "themes.dark.",
        "themes.light.",
        "filters.",
        "archives.",
    ] {
        assert!(text.lines().any(|l| l.starts_with(file)), "{file}");
    }
}

#[test]
fn strict_commands_fail_and_settings_repair() {
    let s = Sandbox::new();
    s.file(
        "cfg/options.toml",
        "[text_editing]\nnumber_of_context_lines = \"many\"\n[tweaks.metrics]\nmax_mru_items = 0\n",
    );
    s.file("a.txt", "a\n");
    for args in [
        vec!["text", "compare", "a.txt", "a.txt"],
        vec!["format", "list"],
        vec!["quick", "a.txt", "a.txt"],
    ] {
        let (c, _, err) = run(&s, &args);
        assert_eq!(c, 100, "{args:?}");
        assert!(
            err.contains("options.toml") && err.contains("text_editing.number_of_context_lines"),
            "{err}"
        );
    }
    let out = s
        .cmd()
        .args(["settings", "list", "--changed", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let codes: Vec<_> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        codes,
        vec!["invalid_config_value", "invalid_config_value"],
        "{doc}"
    );
    let (c, text, err) = run(
        &s,
        &[
            "settings",
            "set",
            "options.text_editing.number_of_context_lines",
            "5",
        ],
    );
    assert_eq!(c, 0, "{err}");
    assert_eq!(text, "options.text_editing.number_of_context_lines = 5\n");
    assert!(
        err.contains("invalid_config_value") || err.contains("the default is used"),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(s.config().join("options.toml")).unwrap(),
        "schema_version = 1\n\n[text_editing]\nnumber_of_context_lines = 5\n",
        "the set repairs the whole file"
    );
    let (c, _, err) = run(&s, &["text", "compare", "a.txt", "a.txt"]);
    assert!(c < 100, "{err}");
    // A newer file stops every command, the settings commands too (SES-052).
    s.file("cfg/options.toml", "schema_version = 9\n");
    let (c, _, err) = run(&s, &["settings", "list"]);
    assert_eq!(c, 100);
    assert!(err.contains("newer"), "{err}");
}

#[test]
fn restore_defaults_plan_and_confirmation() {
    let s = Sandbox::new();
    s.file(
        "cfg/options.toml",
        "[tabs]\nhide_tab_bar_if_single_tab = true\n",
    );
    s.file("cfg/themes/mine.toml", "base = \"dark\"\n");
    s.file(
        "cfg/formats/x.toml",
        "schema_version = 1\nid = \"x\"\nname = \"X\"\nkind = \"text\"\n",
    );
    let (c, _, err) = run(&s, &["settings", "restore-defaults", "--all"]);
    assert_eq!(c, 100);
    assert!(err.contains("--yes"), "{err}");
    let (c, text, _) = run(&s, &["settings", "restore-defaults", "--all", "--dry-run"]);
    assert_eq!(c, 0);
    assert_golden("settings/restore-dry-run.txt", &s.normalise(&text));
    assert!(s.config().join("options.toml").exists());
    let (c, text, _) = run(
        &s,
        &[
            "settings",
            "restore-defaults",
            "--file",
            "options",
            "--file",
            "themes",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(c, 0);
    assert_golden("settings/restore.json", &s.normalise(&text));
    assert!(!s.config().join("options.toml").exists());
    assert!(!s.config().join("themes/mine.toml").exists());
    assert!(
        s.config().join("formats/x.toml").exists(),
        "formats are not touched"
    );
    let (c, _, err) = run(
        &s,
        &["settings", "restore-defaults", "--file", "bogus", "--yes"],
    );
    assert_eq!(c, 100);
    assert!(err.contains("unknown settings file"), "{err}");
    let out = s
        .cmd()
        .args(["settings", "restore-defaults", "--yes", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "a usage error");
    assert_eq!(json(&out)["schema"], "hikasa.usage_error.v1");
}

#[test]
fn help_settings_resolves() {
    let s = Sandbox::new();
    let (c, text, _) = run(&s, &["help", "settings", "set"]);
    assert_eq!(c, 0);
    assert!(
        text.starts_with("hikasa settings") || text.contains("# hikasa settings"),
        "{text}"
    );
}

#[test]
fn restore_defaults_usage_names_every_alternative() {
    let s = Sandbox::new();
    let (c, _, err) = run(&s, &["settings", "restore-defaults", "--dry-run"]);
    assert_eq!(c, 100);
    assert!(
        err.contains("<--all|--file <FILE>|--category <CAT>>"),
        "{err}"
    );
    let (c, _, err) = run(&s, &["format", "restore", "--dry-run"]);
    assert_eq!(c, 100);
    assert!(err.contains("<ID|--all>"), "{err}");
    let (c, _, _) = run(
        &s,
        &[
            "settings",
            "restore-defaults",
            "--all",
            "--file",
            "options",
            "--dry-run",
        ],
    );
    assert_eq!(c, 100, "the alternatives exclude each other");
}
