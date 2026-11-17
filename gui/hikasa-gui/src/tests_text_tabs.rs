//! M2 W11c acceptance A11.23 (D-316, D-317): the text rule tabs of Session Settings, through the models. Each tab shows
//! its P0 keys and nothing else; an edited key written with the Defaults scope, and with *this view only* on a named
//! session, equals `hikasa session edit --set` of it (files compared); loading a session and writing without an edit is
//! the identity (P-TABS-ID, also a property test in `text_tabs.rs`); invalid input keeps OK off with core's message and
//! `(!)`; the element checklist is the union of both panes' formats; *Reset to format* writes `"format"` over an
//! inherited entry and removes an own one; the view relives with its edits kept; `Rules`, the toolbar's *Format* and
//! *Edit Grammar…* open what they open.

use std::collections::BTreeMap;

use hikasa_core::Side;
use hikasa_session::api::{SessionEdit, SessionTarget};
use hikasa_session::sessions::{
    Scope as RuleScope, Session, SessionRef, SessionRules, SessionType,
};

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel as _};
use crate::formats::editor::FormatEditor;
use crate::session_dialog::SessionSettingsDialog;
use crate::tests::World;
use crate::tests_text::Fixture;
use crate::text_tabs::{
    ALIGNMENT, AlignmentTab, FORMAT, FormatTab, IMPORTANCE, ImportanceTab, Origin,
};

const LEFT: &str = "/* head */\nint a = 1;\nint b = 2;\n";
const RIGHT: &str = "/* HEAD */\nint a = 1;\nint b = 3;\n";

fn dialog(f: &mut Fixture) -> &mut SessionSettingsDialog {
    let w = f.app.win();
    f.app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<SessionSettingsDialog>())
        .expect("Session Settings")
}

fn tab<T: crate::session_dialog::SessionTab>(f: &mut Fixture) -> &mut T {
    dialog(f).tab_mut::<T>().expect("the tab")
}

fn rules(f: &mut Fixture) {
    f.run(CommandId::Rules);
    f.settle();
}

fn button(f: &mut Fixture, role: ButtonRole) {
    let w = f.app.win();
    f.app.0.dialog_button(w, role);
    f.settle();
}

/// Every file below the configuration's sessions folder: path → content.
fn sessions_files(world: &World) -> BTreeMap<String, String> {
    let root = world.path("config/sessions");
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.to_string_lossy().contains("_autosaved") {
                let rel = path
                    .strip_prefix(&root)
                    .expect("below")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}

/// One P0 key: its `session edit --set` key and value, and the same edit on the tabs.
type Edit = (&'static str, &'static str, fn(&mut SessionSettingsDialog));

fn edits() -> Vec<Edit> {
    fn a(d: &mut SessionSettingsDialog) -> &mut AlignmentTab {
        d.tab_mut().expect("Alignment")
    }
    fn i(d: &mut SessionSettingsDialog) -> &mut ImportanceTab {
        d.tab_mut().expect("Importance")
    }
    vec![
        ("rules.alignment.algorithm", "\"myers\"", |d| {
            a(d).fields.algorithm = hikasa_text::AlgorithmChoice::Myers
        }),
        ("rules.alignment.skew_tolerance", "40", |d| {
            a(d).fields.skew = "40".into()
        }),
        ("rules.alignment.closeness", "false", |d| {
            a(d).fields.closeness = false
        }),
        ("rules.alignment.closeness_threshold_permille", "700", |d| {
            a(d).fields.threshold = "700".into()
        }),
        ("rules.alignment.never_align_differences", "true", |d| {
            a(d).fields.never_align = true
        }),
        ("rules.importance.leading_whitespace", "false", |d| {
            i(d).fields.leading = false
        }),
        ("rules.importance.embedded_whitespace", "false", |d| {
            i(d).fields.embedded = false
        }),
        ("rules.importance.trailing_whitespace", "false", |d| {
            i(d).fields.trailing = false
        }),
        ("rules.importance.everything_else", "false", |d| {
            i(d).fields.everything_else = false
        }),
        (
            "rules.importance.everything_else_case_sensitive",
            "false",
            |d| i(d).fields.everything_else_case = false,
        ),
        ("rules.importance.orphans_always_important", "false", |d| {
            i(d).fields.orphans = false
        }),
        ("rules.importance.compare_line_endings", "true", |d| {
            i(d).fields.line_endings = true
        }),
        ("rules.importance.elements.Comment", "false", |d| {
            i(d).set_important("Comment", false)
        }),
        (
            "rules.importance.unimportant",
            "[{ pattern = \"x+\", regex = true, match_case = false }]",
            |d| {
                let t = i(d);
                t.pattern = hikasa_text::TextPattern {
                    pattern: "x+".into(),
                    regex: true,
                    match_case: false,
                };
                t.add_pattern();
            },
        ),
    ]
}

#[test]
fn a11_23_rules_opens_importance_and_each_tab_shows_its_p0_keys() {
    let mut f = Fixture::new(LEFT, RIGHT);
    rules(&mut f);
    let d = dialog(&mut f);
    let titles: Vec<String> = d.tabs.iter().map(|t| t.title()).collect();
    assert_eq!(titles, ["Specs", ALIGNMENT, IMPORTANCE, FORMAT]);
    assert_eq!(d.tabs[d.active].title(), IMPORTANCE);
    assert!(
        !d.specs().file_type,
        "the format and encodings are the Format tab's"
    );
    // The tabs' fields are the text rules' keys (P0), nothing else: every field edited writes exactly them.
    let mut r = hikasa_text::TextRules::default();
    let a = tab::<AlignmentTab>(&mut f);
    a.fields.algorithm = hikasa_text::AlgorithmChoice::Patience;
    a.fields.skew = "1".into();
    a.fields.closeness = false;
    a.fields.threshold = "1".into();
    a.fields.never_align = true;
    let i = tab::<ImportanceTab>(&mut f);
    i.set_important("Comment", false);
    i.fields
        .unimportant
        .push(hikasa_text::TextPattern::plain("x"));
    let fl = &mut i.fields;
    for b in [
        &mut fl.leading,
        &mut fl.embedded,
        &mut fl.trailing,
        &mut fl.everything_else,
        &mut fl.everything_else_case,
        &mut fl.orphans,
    ] {
        *b = false;
    }
    fl.line_endings = true;
    let d = dialog(&mut f);
    for t in d.tabs.iter().filter(|t| t.changes_rules()) {
        if let Some(SessionRules::Text(next)) = t.rules(&SessionRules::Text(r.clone())) {
            r = next;
        }
    }
    // The keys that differ from the defaults (D-04), as dotted paths.
    let written = serde_json::to_value(&r).unwrap();
    let default = serde_json::to_value(hikasa_text::TextRules::default()).unwrap();
    let mut keys: Vec<String> = Vec::new();
    for (table, inner) in written.as_object().unwrap() {
        let Some(inner) = inner.as_object() else {
            continue;
        };
        for (key, value) in inner {
            if default[table][key] != *value {
                keys.push(format!("{table}.{key}"));
            }
        }
    }
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "alignment.algorithm",
            "alignment.closeness",
            "alignment.closeness_threshold_permille",
            "alignment.never_align_differences",
            "alignment.skew_tolerance",
            "importance.compare_line_endings",
            "importance.elements",
            "importance.embedded_whitespace",
            "importance.everything_else",
            "importance.everything_else_case_sensitive",
            "importance.leading_whitespace",
            "importance.orphans_always_important",
            "importance.trailing_whitespace",
            "importance.unimportant",
        ],
        "{written}"
    );
    assert_eq!(r.importance.unimportant.len(), 1);
    // The Format tab: per side format and encoding, *Same format for both sides*.
    let fmt = tab::<FormatTab>(&mut f);
    assert!(fmt.fields.same);
    assert!(
        fmt.formats.iter().any(|n| n.starts_with("C, C++")),
        "{:?}",
        fmt.formats
    );
}

#[test]
fn a11_23_an_edit_with_the_defaults_scope_is_session_edit_set() {
    for (key, value, edit) in edits() {
        let twin = World::new();
        let mut f = Fixture::new(LEFT, RIGHT);
        rules(&mut f);
        edit(dialog(&mut f));
        let d = dialog(&mut f);
        assert!(d.scope_available(RuleScope::Defaults), "{key}");
        d.scope = RuleScope::Defaults;
        button(&mut f, ButtonRole::Ok);
        assert!(
            f.app.window(f.app.win()).unwrap().dialogs.is_empty(),
            "{key}: OK closed"
        );
        twin.service
            .session_edit(
                &SessionTarget::Defaults(SessionType::TextCompare),
                &SessionEdit {
                    set: vec![(key.to_owned(), value.to_owned())],
                    ..SessionEdit::default()
                },
                false,
            )
            .expect("session edit");
        assert_eq!(sessions_files(&f.world), sessions_files(&twin), "{key}");
    }
}

/// A named Text Compare session "main" of the fixture's files and its view.
fn named(world: World) -> Fixture {
    let l = world.write("work/left.c", LEFT);
    let r = world.write("work/right.c", RIGHT);
    // A new session takes the defaults as its template (SES-001).
    let defaults = world
        .service
        .session_defaults(SessionType::TextCompare)
        .expect("defaults");
    let mut s = Session::new(SessionType::TextCompare, &defaults);
    s.specs.left = l.display().to_string();
    s.specs.right = r.display().to_string();
    world
        .service
        .session_save(&SessionRef::parse("main").unwrap(), &s, false)
        .expect("save");
    let mut f = Fixture::open_paths(world, l, r);
    // The fixture's own view closes; the named one opens.
    let w = f.app.win();
    f.app.0.open_named(
        SessionRef::parse("main").unwrap(),
        crate::app::OpenHow::Target(crate::app::OpenTarget::Window(w)),
    );
    f.settle();
    f.view = f.app.active_tab().expect("a tab").id;
    f
}

/// The `[rules…]` sections of a session file, as written.
fn rules_table(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with('[') {
            inside = line.starts_with("[rules");
        }
        if inside && !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn a11_23_an_edit_for_this_view_only_is_session_edit_set_of_the_named_session() {
    for (key, value, edit) in edits() {
        let mut f = named(World::new());
        rules(&mut f);
        edit(dialog(&mut f));
        dialog(&mut f).scope = RuleScope::View;
        button(&mut f, ButtonRole::Ok);
        f.run(CommandId::SessionSave);
        f.settle();
        let mine = std::fs::read_to_string(f.world.path("config/sessions/main.toml")).unwrap();
        let twin = named(World::new());
        twin.world
            .service
            .session_edit(
                &SessionTarget::Named(SessionRef::parse("main").unwrap()),
                &SessionEdit {
                    set: vec![(key.to_owned(), value.to_owned())],
                    ..SessionEdit::default()
                },
                false,
            )
            .expect("session edit");
        let theirs = std::fs::read_to_string(twin.world.path("config/sessions/main.toml")).unwrap();
        assert_eq!(
            rules_table(&mine),
            rules_table(&theirs),
            "{key}\n{mine}\n{theirs}"
        );
    }
}

/// P-TABS-ID through the dialog: a session with a value for every key, OK without an edit: the file stays as it was.
#[test]
fn a11_23_ok_without_an_edit_is_the_identity() {
    let world = World::new();
    let mut defaults = world
        .service
        .session_defaults(SessionType::TextCompare)
        .unwrap();
    if let SessionRules::Text(r) = &mut defaults.rules {
        r.alignment.skew_tolerance = 9;
        r.importance.elements.insert("String".into(), false);
    }
    world.service.session_save_defaults(&defaults).unwrap();
    let mut f = named(world);
    let before = sessions_files(&f.world);
    rules(&mut f);
    assert!(dialog(&mut f).tabs.iter().all(|t| !t.changes_rules()));
    dialog(&mut f).scope = RuleScope::View;
    button(&mut f, ButtonRole::Ok);
    f.run(CommandId::SessionSave);
    f.settle();
    let after = sessions_files(&f.world);
    assert_eq!(
        rules_table(&before["main.toml"]),
        rules_table(&after["main.toml"])
    );
    assert_eq!(
        before["_defaults/text-compare.toml"],
        after["_defaults/text-compare.toml"]
    );
}

#[test]
fn a11_23_invalid_input_keeps_ok_off_with_cores_message() {
    let mut f = Fixture::new(LEFT, RIGHT);
    rules(&mut f);
    type Bad = (&'static str, &'static str, fn(&mut SessionSettingsDialog));
    let cases: [Bad; 4] = [
        (IMPORTANCE, "unimportant[0]", |d| {
            let t: &mut ImportanceTab = d.tab_mut().unwrap();
            t.fields
                .unimportant
                .push(hikasa_text::TextPattern::regex("(unclosed"));
        }),
        (ALIGNMENT, "1001 is not in 0..=1000", |d| {
            d.tab_mut::<AlignmentTab>().unwrap().fields.threshold = "1001".into();
        }),
        (ALIGNMENT, "whole number", |d| {
            d.tab_mut::<AlignmentTab>().unwrap().fields.skew = "lots".into();
        }),
        (IMPORTANCE, "reserved", |d| {
            d.tab_mut::<ImportanceTab>()
                .unwrap()
                .set_important("Everything Else", false);
        }),
    ];
    for (title, message, edit) in cases {
        rules(&mut f);
        let svc = f.world.service.clone();
        let d = dialog(&mut f);
        edit(d);
        d.poll(&svc);
        let (k, error) = d.tab_error().expect("an error");
        assert_eq!(d.tabs[k].title(), title);
        assert!(error.contains(message), "{error}");
        assert_eq!(d.tab_caption(k), format!("{title} (!)"));
        assert!(!d.buttons()[0].enabled, "OK is off");
        button(&mut f, ButtonRole::Cancel);
    }
}

#[test]
fn a11_23_the_checklist_is_the_union_of_both_panes_formats() {
    let mut f = Fixture::named("/* c */ int x;\n", "# py\nx = 1\n", "left.c", "right.py");
    // The right pane's own format (D-281): the panes use two formats.
    f.m()
        .request_format(Side::Right, Some("Python Scripts".into()));
    f.settle();
    assert_eq!(
        crate::text::info::formats_used(f.m()),
        ["C, C++, C#, ObjC Source", "Python Scripts"]
    );
    rules(&mut f);
    let svc = f.world.service.clone();
    let mut want: Vec<String> = Vec::new();
    for id in ["c-cpp", "python"] {
        let format = svc.format_for_edit(id).unwrap().format;
        for n in format
            .grammar
            .iter()
            .map(|g| g.element.as_str().to_owned())
            .chain(format.elements.keys().map(|k| k.as_str().to_owned()))
        {
            if !want.contains(&n) {
                want.push(n);
            }
        }
    }
    let rows: Vec<String> = tab::<ImportanceTab>(&mut f)
        .rows()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(rows, want);
    // Python's whitespace element ships unimportant: the checklist shows the format's default, from the format.
    let unimportant: Vec<_> = tab::<ImportanceTab>(&mut f)
        .rows()
        .into_iter()
        .filter(|r| !r.important)
        .collect();
    assert_eq!(unimportant.len(), 1, "{unimportant:?}");
    assert_eq!(unimportant[0].origin, Origin::Format);
}

#[test]
fn a11_23_reset_to_format_writes_format_over_an_inherited_entry_and_removes_an_own_one() {
    let world = World::new();
    world
        .service
        .session_edit(
            &SessionTarget::Defaults(SessionType::TextCompare),
            &SessionEdit {
                set: vec![("rules.importance.elements.Comment".into(), "false".into())],
                ..SessionEdit::default()
            },
            false,
        )
        .unwrap();
    let mut f = named(world);
    rules(&mut f);
    let t = tab::<ImportanceTab>(&mut f);
    let comment = t.rows().into_iter().find(|r| r.name == "Comment").unwrap();
    assert_eq!(
        (comment.important, comment.origin),
        (false, Origin::Defaults)
    );
    t.set_important("String", false);
    assert_eq!(
        t.rows()
            .into_iter()
            .find(|r| r.name == "String")
            .unwrap()
            .origin,
        Origin::Session
    );
    t.reset_to_format("Comment");
    dialog(&mut f).scope = RuleScope::View;
    button(&mut f, ButtonRole::Ok);
    f.run(CommandId::SessionSave);
    f.settle();
    let text = std::fs::read_to_string(f.world.path("config/sessions/main.toml")).unwrap();
    assert!(text.contains("Comment = \"format\""), "{text}");
    assert!(text.contains("String = false"), "{text}");
    // The own entry: its reset removes it.
    rules(&mut f);
    tab::<ImportanceTab>(&mut f).reset_to_format("String");
    dialog(&mut f).scope = RuleScope::View;
    button(&mut f, ButtonRole::Ok);
    f.run(CommandId::SessionSave);
    f.settle();
    let text = std::fs::read_to_string(f.world.path("config/sessions/main.toml")).unwrap();
    assert!(!text.contains("String"), "{text}");
    assert!(text.contains("Comment = \"format\""), "{text}");
    // The comparison took it: the view's rules hold no Comment entry.
    let base = f.m().request().base_rules.clone().unwrap();
    assert!(
        base.importance.elements.is_empty(),
        "{:?}",
        base.importance.elements
    );
}

#[test]
fn a11_23_the_view_relives_with_its_edits_kept() {
    let mut f = Fixture::new(LEFT, RIGHT);
    f.place(Side::Left, 1, 0);
    f.type_text("zzz");
    f.settle();
    let before = f.text(Side::Left);
    assert!(before.contains("zzzint a"));
    rules(&mut f);
    tab::<AlignmentTab>(&mut f).fields.never_align = true;
    tab::<ImportanceTab>(&mut f).fields.everything_else_case = false;
    button(&mut f, ButtonRole::Ok);
    assert_eq!(f.text(Side::Left), before, "the edits stay");
    assert!(f.doc(Side::Left).is_modified());
    let base = f.m().request().base_rules.clone().expect("the session's");
    assert!(base.alignment.never_align_differences);
    assert!(!base.importance.everything_else_case_sensitive);
    // A format picked on the Format tab relives too (the left pane's own format).
    rules(&mut f);
    let fmt = tab::<FormatTab>(&mut f);
    fmt.fields.same = false;
    fmt.fields.left = "Java Source".into();
    button(&mut f, ButtonRole::Ok);
    assert_eq!(f.text(Side::Left), before);
    assert_eq!(f.m().request().formats.left.as_deref(), Some("Java Source"));
}

#[test]
fn a11_23_format_and_edit_grammar_open_the_format_editor() {
    let mut f = Fixture::new(LEFT, RIGHT);
    // The toolbar's Format: the editor of the focus pane's format.
    f.run(CommandId::FileFormat);
    f.settle();
    let w = f.app.win();
    let id = f
        .app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<FormatEditor>())
        .map(|e| (e.id.clone(), e.ready()));
    assert_eq!(id, Some(("c-cpp".to_owned(), true)));
    button(&mut f, ButtonRole::Cancel);
    // Edit Grammar… of the Importance tab.
    rules(&mut f);
    tab::<ImportanceTab>(&mut f).press_edit_grammar();
    f.settle();
    let editor = f
        .app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<FormatEditor>())
        .map(|e| e.id.clone());
    assert_eq!(editor.as_deref(), Some("c-cpp"));
    assert_eq!(
        f.app.0.window(w).unwrap().dialogs.len(),
        2,
        "over the settings"
    );
    // F1 shows each dialog's topic.
    assert_eq!(
        f.app
            .0
            .window(w)
            .unwrap()
            .dialogs
            .top()
            .unwrap()
            .help_topic(),
        "dialog-format-editor"
    );
}
