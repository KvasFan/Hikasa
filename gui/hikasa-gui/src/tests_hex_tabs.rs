//! A13.22 (M2 W13a, D-370): the rule tabs and sessions of a Hex Compare view — each Alignment field writes the key
//! `session edit --set` writes (the defaults scope); P-TABS-ID: open and OK without edits writes nothing, random edits
//! write only their keys; refused words keep OK off with the core's message; OK relives the view (a realign without a
//! reopen); the view state (`[view]`, `[display]`, the new keys included) round trips through Save Session and a
//! reopen and a workspace; a pre-W13 session without the new keys loads with their defaults. Mutation: the Alignment
//! tab writes `window` when only `block` changed.

use std::collections::BTreeMap;

use hikasa_session::api::hex::{HexRulesFile, HexSessionRules};
use hikasa_session::api::{SessionEdit, SessionTarget};
use hikasa_session::sessions::{Scope as RuleScope, SessionRef, SessionRules, SessionType};
use proptest::prelude::*;

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::hex_tabs::{ALIGNMENT, AlignmentTab};
use crate::session_dialog::{SessionSettingsDialog, SessionTab};
use crate::tests::{TestApp, World};
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, with_insert};

fn dialog(app: &mut TestApp) -> &mut SessionSettingsDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<SessionSettingsDialog>())
        .expect("Session Settings")
}

/// The `[rules…]` and `_defaults` files below the sessions folder.
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

/// One Alignment field: its `session edit --set` key and value, and the same edit on the tab.
type Edit = (&'static str, &'static str, fn(&mut AlignmentTab));

fn edits() -> Vec<Edit> {
    vec![
        ("rules.hex.alignment", "complete", |t| {
            t.fields.mode = "complete".into()
        }),
        ("rules.hex.block", "4096", |t| {
            t.fields.block = "4096".into()
        }),
        ("rules.hex.skew", "1048576", |t| {
            t.fields.skew = "1MiB".into()
        }),
        ("rules.hex.window", "65536", |t| {
            t.fields.window = "64k".into()
        }),
        ("rules.hex.budget_mib", "64", |t| {
            t.fields.budget_mib = "64".into()
        }),
        ("rules.hex.unlocked_max_mib", "4", |t| {
            t.fields.unlocked_max_mib = "4".into()
        }),
    ]
}

fn open(world: &World, app: &mut TestApp) -> hikasa_session::ipc::ViewId {
    let a = bytes(101, 6000);
    let b = with_insert(&a, 300, 5, 4000);
    open_pair(world, app, &a, &b)
}

/// A13.22: `Rules` opens the dialog on the Alignment tab; each field with the defaults scope writes what `session edit
/// --set` writes.
#[test]
fn a13_22_each_field_is_session_edit_set() {
    for (key, value, edit) in edits() {
        let world = hex_world();
        let twin = hex_world();
        let mut app = world.app(None);
        open(&world, &mut app);
        app.execute(CommandId::Rules);
        world.settle(&mut app);
        let d = dialog(&mut app);
        assert_eq!(d.tabs[d.active].title(), ALIGNMENT);
        edit(d.tab_mut::<AlignmentTab>().expect("tab"));
        d.scope = RuleScope::Defaults;
        world.settle(&mut app);
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Ok);
        world.settle(&mut app);
        twin.service
            .session_edit(
                &SessionTarget::Defaults(SessionType::HexCompare),
                &SessionEdit {
                    set: vec![(key.to_owned(), value.to_owned())],
                    ..SessionEdit::default()
                },
                false,
            )
            .expect("session edit");
        assert_eq!(sessions_files(&world), sessions_files(&twin), "{key}");
    }
}

/// A13.22: refused words keep OK off with the core's message.
#[test]
fn a13_22_refused_words() {
    let world = hex_world();
    let mut app = world.app(None);
    open(&world, &mut app);
    app.execute(CommandId::Rules);
    world.settle(&mut app);
    for (bad, set) in [
        (
            "slow",
            (|t: &mut AlignmentTab| t.fields.mode = "slow".into()) as fn(&mut AlignmentTab),
        ),
        ("1000", |t| t.fields.block = "1000".into()),
        ("2000", |t| t.fields.unlocked_max_mib = "2000".into()),
    ] {
        let d = dialog(&mut app);
        let t = d.tab_mut::<AlignmentTab>().expect("tab");
        let saved = t.fields.clone();
        set(t);
        world.settle(&mut app);
        let d = dialog(&mut app);
        let t = d.tab_mut::<AlignmentTab>().expect("tab");
        let e = t.error().map(str::to_owned).expect("refused");
        assert!(e.contains(bad), "{e}");
        assert!(
            !d.buttons()
                .iter()
                .any(|b| b.role == ButtonRole::Ok && b.enabled),
            "OK off"
        );
        dialog(&mut app)
            .tab_mut::<AlignmentTab>()
            .expect("tab")
            .fields = saved;
        world.settle(&mut app);
    }
}

/// A13.22: OK of a rule change relives the view: a realign without a reopen; the mode becomes the view's.
#[test]
fn a13_22_ok_realigns_without_reopening() {
    let world = hex_world();
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = hex_model(&mut app, view).counters;
    app.execute(CommandId::Rules);
    world.settle(&mut app);
    dialog(&mut app)
        .tab_mut::<AlignmentTab>()
        .expect("tab")
        .fields
        .mode = "none".into();
    dialog(&mut app).scope = RuleScope::View;
    world.settle(&mut app);
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.opens, before.opens, "no reopen");
    assert_eq!(m.counters.aligns, before.aligns + 1);
    assert_eq!(
        m.alignment().expect("landed").mode,
        hikasa_hex::AlignMode::None
    );
}

/// A13.22: the view state round trips through Save Session and a reopen (every `[view]` and `[display]` key, the new
/// ones included); a pre-W13 session without them loads with their defaults.
#[test]
fn a13_22_view_state_round_trip() {
    let world = hex_world();
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let m = hex_model(&mut app, view);
    let mut layout = m.display().layout;
    layout.bytes_per_line = 24;
    layout.group = 4;
    layout.chars = hikasa_hex::CharSet::Latin1;
    layout.address = hikasa_hex::Radix::Decimal;
    m.set_layout(layout, false, crate::hex::PaneLayout::OverUnder, 5);
    m.set_filter(hikasa_hex::DisplayFilter::Same);
    m.set_values(hikasa_session::api::HexValues::Big);
    m.set_values_radix(hikasa_hex::Radix::Hex);
    app.execute(CommandId::ShowThumbnail);
    // Save as a named session, then open it in a new tab.
    let tab = app.tab(view).expect("tab");
    let capture = crate::app::AppCore::capture_for_test(tab).expect("capture");
    let (session, _) = capture.session(&world.service).expect("session");
    let r = SessionRef::parse("hx").expect("name");
    world
        .service
        .session_save(&r, &session, false)
        .expect("save");
    let text = std::fs::read_to_string(world.path("config/sessions/hx.toml")).expect("file");
    for key in [
        "bytes_per_line = 24",
        "group = 4",
        "chars = \"latin1\"",
        "address = \"decimal\"",
        "context = 5",
        "addresses = false",
        "thumbnail = false",
        "values = \"big\"",
        "values_radix = \"hex\"",
        "filter = \"same\"",
    ] {
        assert!(text.contains(key), "{key} in\n{text}");
    }
    let w = app.win();
    let index = app.active();
    app.close_tab(index, hikasa_session::ipc::CloseReason::User);
    world.settle(&mut app);
    app.0.open_named(
        r,
        crate::app::OpenHow::Target(crate::app::OpenTarget::Window(w)),
    );
    world.settle(&mut app);
    let reopened = app.active_tab().expect("tab").id;
    assert_ne!(reopened, view);
    let m = hex_model(&mut app, reopened);
    let d = *m.display();
    assert_eq!(d.layout, layout);
    assert_eq!(d.context, 5);
    assert!(!d.addresses && !d.thumbnail);
    assert_eq!(d.panes, crate::hex::PaneLayout::OverUnder);
    assert_eq!(d.filter, hikasa_hex::DisplayFilter::Same);
    assert_eq!(d.values, hikasa_session::api::HexValues::Big);
    assert_eq!(d.values_radix, hikasa_hex::Radix::Hex);
    // A pre-W13 session: no new key, their defaults.
    let old = text
        .lines()
        .filter(|l| {
            ![
                "addresses",
                "thumbnail",
                "values",
                "values_radix",
                "layout = ",
            ]
            .iter()
            .any(|k| l.starts_with(k))
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(world.path("config/sessions/old.toml"), old).expect("write");
    app.0.open_named(
        SessionRef::parse("old").expect("name"),
        crate::app::OpenHow::Target(crate::app::OpenTarget::Window(w)),
    );
    world.settle(&mut app);
    let v = app.active_tab().expect("tab").id;
    let d = *hex_model(&mut app, v).display();
    assert!(d.addresses && d.thumbnail);
    assert_eq!(d.values, hikasa_session::api::HexValues::None);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(64),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// P-TABS-ID for the Alignment tab: loaded and left untouched it writes the rules unchanged; random edits change
    /// exactly their keys, to the values core parses.
    #[test]
    fn p_tabs_id_alignment(
        mode in prop::sample::select(
            std::iter::once("").chain(hikasa_hex::AlignMode::ALL.iter().map(|m| m.as_str())).collect::<Vec<_>>()
        ),
        block in prop::sample::select(vec!["", "1024", "8k", "1MiB"]),
        skew in prop::sample::select(vec!["", "4096", "1MiB", "16MiB"]),
        window in prop::sample::select(vec!["", "4k", "65536", "16MiB"]),
        budget in prop::sample::select(vec!["", "1", "64", "1024"]),
        unlocked in prop::sample::select(vec!["", "0", "16", "1024"]),
        start_block in prop::sample::select(vec![1024u64, 8192, 65536]),
        start_window in prop::sample::select(vec![65536u64, 262144, 1 << 20]),
    ) {
        let current = SessionRules::Hex(Box::new(HexSessionRules {
            hex: HexRulesFile { block: start_block, window: start_window, ..HexRulesFile::default() },
        }));
        let mut tab = AlignmentTab::default();
        tab.load(&current);
        prop_assert!(!tab.changes_rules());
        prop_assert_eq!(tab.rules(&current), Some(current.clone()));
        let before = match &current { SessionRules::Hex(h) => h.hex.clone(), _ => unreachable!() };
        let mut changed = Vec::new();
        if !mode.is_empty() { tab.fields.mode = mode.into(); changed.push("alignment"); }
        if !block.is_empty() { tab.fields.block = block.into(); changed.push("block"); }
        if !skew.is_empty() { tab.fields.skew = skew.into(); changed.push("skew"); }
        if !window.is_empty() { tab.fields.window = window.into(); changed.push("window"); }
        if !budget.is_empty() { tab.fields.budget_mib = budget.into(); changed.push("budget_mib"); }
        if !unlocked.is_empty() { tab.fields.unlocked_max_mib = unlocked.into(); changed.push("unlocked_max_mib"); }
        prop_assume!(HexRulesFile::of_words(&tab_words(&tab)).is_ok());
        let after = match tab.rules(&current) {
            Some(SessionRules::Hex(h)) => h.hex,
            other => panic!("{other:?}"),
        };
        let fields = |f: &HexRulesFile| -> Vec<(&'static str, String)> { vec![
            ("alignment", f.alignment.clone()),
            ("block", f.block.to_string()),
            ("skew", f.skew.to_string()),
            ("window", f.window.to_string()),
            ("budget_mib", f.budget_mib.to_string()),
            ("unlocked_max_mib", f.unlocked_max_mib.to_string()),
        ]};
        for ((key, b), (_, a)) in fields(&before).into_iter().zip(fields(&after)) {
            if !changed.contains(&key) {
                prop_assert_eq!(&b, &a, "{} unchanged", key);
            }
        }
        // The changed keys are what core parses of the words.
        let parsed = HexRulesFile::of_words(&tab_words(&tab)).expect("valid words");
        for ((key, p), (_, a)) in fields(&parsed).into_iter().zip(fields(&after)) {
            if changed.contains(&key) {
                prop_assert_eq!(&p, &a, "{} parsed", key);
            }
        }
    }
}

/// The tab's fields as rule words (every field).
fn tab_words(tab: &AlignmentTab) -> hikasa_session::api::HexRuleWords {
    let w = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    hikasa_session::api::HexRuleWords {
        alignment: w(&tab.fields.mode),
        block: w(&tab.fields.block),
        skew: w(&tab.fields.skew),
        window: w(&tab.fields.window),
        budget_mib: w(&tab.fields.budget_mib),
        unlocked_max_mib: w(&tab.fields.unlocked_max_mib),
    }
}
