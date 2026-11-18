//! A11.22 (D-315): Info's table for a fixture (*provisional* while edits wait); Clear Session asks D-301 and leaves an
//! empty session whose headers' paths open it; an input changed on disk with edits gives *Reload* / *Keep mine*, never a
//! reload; a save's own write raises nothing; a format file changed on disk offers *Reopen*, which relives a modified
//! view and reloads an unmodified one.

use hikasa_core::Side;
use hikasa_session::settings::{ChangeKind, ConfigChange};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::info_dialog::InfoDialog;
use crate::notify::NoticeAction;
use crate::tests_text::Fixture;
use crate::text::info;
use crate::watch::WatchEvent;

const L: &str = "alpha\nbeta\ngamma\n";
const R: &str = "alpha\nBETA\ngamma\r\n";

/// The value of `label` in a section of the table (first column).
fn value(sections: &[crate::info_dialog::InfoSection], label: &str) -> Vec<String> {
    sections
        .iter()
        .flat_map(|s| &s.rows)
        .find(|r| r.label == label)
        .map(|r| r.values.clone())
        .unwrap_or_default()
}

#[test]
fn a11_22_the_info_table() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Info);
    let win = f.app.win();
    let sections = {
        let w = f.app.window_mut(win).expect("window");
        let d = w.dialogs.find_mut::<InfoDialog>().expect("Info");
        assert_eq!(crate::dialog::DialogModel::title(d), "Text Compare Info");
        assert_eq!(
            crate::dialog::DialogModel::help_topic(d),
            info::TEXT_INFO_TOPIC
        );
        d.sections()
    };
    assert_eq!(sections, info::sections(f.m()));
    assert_eq!(value(&sections, "Lines"), vec!["3", "3"]);
    assert_eq!(value(&sections, "Encoding"), vec!["UTF-8", "UTF-8"]);
    assert_eq!(
        value(&sections, "Line endings"),
        vec!["LF (3 LF)".to_owned(), "MIX (2 LF, 1 CRLF)".to_owned()]
    );
    assert_eq!(value(&sections, "Modified"), vec!["No", "No"]);
    assert_eq!(
        value(&sections, "Size on disk"),
        vec!["17 bytes", "18 bytes"]
    );
    let stats = f.m().comparison().expect("cmp").stats();
    assert_eq!(value(&sections, "Rows"), vec![stats.rows.to_string()]);
    assert_eq!(
        value(&sections, "Changed rows"),
        vec![stats.changed_rows.to_string()]
    );
    assert!(value(&sections, "State").is_empty(), "installed: exact");
    // While an edit waits for its recompare: *provisional*.
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    let now = info::sections(f.m());
    assert_eq!(value(&now, "Modified"), vec!["Yes", "No"]);
    assert_eq!(
        value(&now, "State"),
        vec!["provisional: edits wait for the comparison".to_owned()]
    );
}

/// The watch's event of a change of `spec`, with the state it has now (what a poll sees).
pub(crate) fn changed(f: &Fixture, spec: &str) -> WatchEvent {
    WatchEvent::Inputs(vec![(
        spec.to_owned(),
        ChangeKind::Changed,
        f.world.service.input_stamp(spec),
    )])
}

#[test]
fn a11_22_clear_asks_first_and_leaves_an_empty_session() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Left, 0, 0);
    f.type_text("x");
    f.run(CommandId::SessionClear);
    assert!(
        f.prompt().is_some_and(|p| p.starts_with("Save changes to")),
        "{:?}",
        f.prompt()
    );
    f.answer(ButtonRole::No);
    assert!(f.m().is_empty_session());
    assert!(f.m().doc(Side::Left).is_none());
    assert!(f.m().meta().message.starts_with("Type the files' paths"));
    // The headers' paths: the view opens once both are given.
    let (l, r) = (f.paths[0].clone(), f.paths[1].clone());
    f.m()
        .request_reopen(Side::Left, Some(l.into_os_string()), None);
    f.settle();
    assert!(f.m().is_empty_session(), "one path is not a comparison");
    f.m()
        .request_reopen(Side::Right, Some(r.into_os_string()), None);
    f.settle();
    assert!(!f.m().is_empty_session());
    assert_eq!(f.text(Side::Left), L);
    assert_eq!(f.text(Side::Right), R);
}

#[test]
fn a11_22_changes_on_disk_with_edits_ask() {
    let mut f = Fixture::new(L, R);
    let spec = f.m().inputs()[0].clone();
    f.place(Side::Left, 0, 0);
    f.type_text("mine ");
    std::fs::write(&f.paths[0], "theirs\n").expect("write");
    let event = changed(&f, &spec);
    f.app.0.on_watch(vec![event]);
    f.settle();
    // Never a reload: the edits stay.
    assert!(f.text(Side::Left).starts_with("mine "));
    let win = f.app.win();
    let key = format!("disk:0:{}", f.view.0);
    let notice = f
        .app
        .window(win)
        .and_then(|w| w.notices.get(&key))
        .cloned()
        .expect("the notice");
    assert_eq!(notice.text, "left.c changed on disk.");
    assert_eq!(
        notice.actions,
        vec![
            NoticeAction::DiskReload(f.view, Side::Left),
            NoticeAction::DiskKeep(f.view, Side::Left)
        ]
    );
    // Keep mine: the notice goes, the edits stay.
    f.app
        .0
        .notice_action(win, &key, NoticeAction::DiskKeep(f.view, Side::Left));
    assert!(f.text(Side::Left).starts_with("mine "));
    // Reload asks D-301 first, then reads the file again.
    let event = changed(&f, &spec);
    f.app.0.on_watch(vec![event]);
    f.app
        .0
        .notice_action(win, &key, NoticeAction::DiskReload(f.view, Side::Left));
    f.settle();
    assert!(f.prompt().is_some(), "D-301");
    f.answer(ButtonRole::No);
    assert_eq!(f.text(Side::Left), "theirs\n");
    // Without edits: D-118 as before (automatically reloaded).
    std::fs::write(&f.paths[1], "right now\n").expect("write");
    let right = f.m().inputs()[1].clone();
    let event = changed(&f, &right);
    f.app.0.on_watch(vec![event]);
    f.settle();
    assert_eq!(f.text(Side::Right), "right now\n");
}

#[test]
fn a11_22_a_saves_own_write_raises_nothing() {
    let mut f = Fixture::new(L, R);
    let spec = f.m().inputs()[0].clone();
    f.place(Side::Left, 0, 0);
    f.type_text("saved ");
    f.run(CommandId::Save);
    f.settle();
    assert!(!f.doc(Side::Left).is_modified());
    let win = f.app.win();
    let before = f.app.window(win).map_or(0, |w| w.notices.all().len());
    let event = changed(&f, &spec);
    f.app.0.on_watch(vec![event]);
    f.settle();
    let after: Vec<String> = f
        .app
        .window(win)
        .map(|w| w.notices.all().iter().map(|n| n.text.clone()).collect())
        .unwrap_or_default();
    assert_eq!(after.len(), before, "{after:?}");
    assert!(f.text(Side::Left).starts_with("saved "));
}

#[test]
fn a11_22_a_format_changed_on_disk_offers_reopen() {
    let mut f = Fixture::new(L, R);
    let name = f.m().response().expect("response").format.name.clone();
    let id = f
        .world
        .service
        .format_list()
        .formats
        .into_iter()
        .find(|x| x.name == name)
        .map(|x| x.id)
        .expect("the format");
    let path = f.world.path(&format!("config/formats/{id}.toml"));
    f.app.0.on_watch(vec![WatchEvent::Config(vec![ConfigChange {
        path,
        kind: ChangeKind::Changed,
    }])]);
    f.settle();
    let win = f.app.win();
    let key = format!("format:{}:{name}", f.view.0);
    let notice = f
        .app
        .window(win)
        .and_then(|w| w.notices.get(&key))
        .cloned()
        .expect("the notice");
    assert_eq!(notice.text, format!("File format {name} changed on disk."));
    assert_eq!(notice.actions, vec![NoticeAction::FormatReopen(f.view)]);
    // Unmodified: Reopen reloads (a new comparison is installed).
    let before = f.m().installs();
    f.app
        .0
        .notice_action(win, &key, NoticeAction::FormatReopen(f.view));
    f.settle();
    assert_eq!(f.m().installs(), before + 1);
    // Modified: a relive keeps the edits.
    f.place(Side::Left, 0, 0);
    f.type_text("kept ");
    f.settle();
    f.app.0.on_watch(vec![WatchEvent::Config(vec![ConfigChange {
        path: f.world.path(&format!("config/formats/{id}.toml")),
        kind: ChangeKind::Changed,
    }])]);
    f.settle();
    f.app
        .0
        .notice_action(win, &key, NoticeAction::FormatReopen(f.view));
    f.settle();
    assert!(f.text(Side::Left).starts_with("kept "));
    assert!(f.doc(Side::Left).can_undo(), "the undo log kept");
}

// ---- W11b review fixes ------------------------------------------------------------------------------------------

/// 10e: Info's Engine row is the engine's name as the views show it.
#[test]
fn w11b_review_the_engine_row_is_a_name() {
    let mut f = Fixture::new(L, R);
    let engine = f.m().comparison().expect("cmp").algorithm();
    let sections = info::sections(f.m());
    assert_eq!(
        value(&sections, "Engine"),
        vec![crate::text::engine_name(engine).to_owned()]
    );
    assert!(hikasa_text::Algorithm::ALL.iter().all(|a| {
        crate::text::engine_name(*a)
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
    }));
}

/// Finding 6: Clear Session gives an empty session of the view's type: nothing of the cleared session (a forced
/// encoding, a format pick) decodes the next files.
#[test]
fn w11b_review_clear_session_starts_from_the_defaults() {
    let mut f = Fixture::new("caf\u{e9}\n", "x\n");
    f.m()
        .request_reopen(Side::Left, None, Some(Some("windows-1252".into())));
    f.settle();
    f.m().request_format(Side::Right, Some("cobol".into()));
    f.settle();
    assert_eq!(
        f.text(Side::Left),
        "caf\u{c3}\u{a9}\n",
        "forced 1252 decode"
    );
    assert!(f.m().request().formats.right.is_some());
    f.run(CommandId::SessionClear);
    f.settle();
    assert!(f.m().is_empty_session());
    let defaults = match f
        .world
        .service
        .session_view_of(
            &{
                let mut s = f
                    .world
                    .service
                    .session_defaults(hikasa_session::sessions::SessionType::TextCompare)
                    .expect("defaults");
                s.specs.left = "clipboard://".into();
                s.specs.right = "clipboard://".into();
                s
            },
            String::new(),
        )
        .expect("view")
        .request
    {
        hikasa_session::api::ViewRequest::Text { request, .. } => request,
        other => panic!("{other:?}"),
    };
    let r = f.m().request().clone();
    assert_eq!(r.encodings, defaults.encodings);
    assert_eq!(r.formats, defaults.formats);
    assert_eq!(r.base_rules, defaults.base_rules);
    assert_eq!(r.rules_patch, defaults.rules_patch);
    assert_eq!(r.ignored_check, Default::default());
    let (l, rp) = (f.paths[0].clone(), f.paths[1].clone());
    f.m()
        .request_reopen(Side::Left, Some(l.into_os_string()), None);
    f.settle();
    f.m()
        .request_reopen(Side::Right, Some(rp.into_os_string()), None);
    f.settle();
    assert!(!f.m().is_empty_session());
    assert_eq!(
        f.text(Side::Left),
        "caf\u{e9}\n",
        "decoded as the defaults say"
    );
}

/// Finding 3 (C32): a save's marker swallows only the watch's event of the state the save left; the first event of
/// the input ends it either way, so every later change is reported.
#[test]
fn w11b_review_a_saves_marker_swallows_its_own_state_only() {
    let notices = |f: &mut Fixture| {
        let win = f.app.win();
        f.app.window(win).map_or(0, |w| w.notices.all().len())
    };
    // The acknowledgement came first (no event for the save's write): the next real change is reported.
    let effect = |saved: bool| {
        let mut f = Fixture::new("a\nb\n", "a\nc\n");
        let spec = f.m().inputs()[0].clone();
        if saved {
            f.place(Side::Left, 0, 0);
            f.type_text("saved ");
            f.run(CommandId::Save);
            f.settle();
            assert!(!f.doc(Side::Left).is_modified());
        }
        let before = notices(&mut f);
        let text = f.text(Side::Left);
        std::fs::write(&f.paths[0], "theirs, longer\n").expect("write");
        let event = changed(&f, &spec);
        f.app.0.on_watch(vec![event]);
        f.settle();
        (notices(&mut f) > before, f.text(Side::Left) != text)
    };
    let baseline = effect(false);
    assert_ne!(baseline, (false, false), "the change shows");
    assert_eq!(
        effect(true),
        baseline,
        "after a save, the next change is reported"
    );
    // The save's own event (its state) is swallowed once; a second event of that state is reported.
    let mut f = Fixture::new("a\nb\n", "a\nc\n");
    let spec = f.m().inputs()[0].clone();
    f.place(Side::Left, 0, 0);
    f.type_text("saved ");
    f.run(CommandId::Save);
    f.settle();
    let own = changed(&f, &spec);
    let before = (notices(&mut f), f.m().installs());
    f.app.0.on_watch(vec![own.clone()]);
    f.settle();
    assert_eq!(
        (notices(&mut f), f.m().installs()),
        before,
        "the save's own write raises nothing"
    );
    f.app.0.on_watch(vec![own]);
    f.settle();
    assert!(
        notices(&mut f) > before.0 || f.m().installs() > before.1,
        "a second event of that state is no longer the save's"
    );
}
