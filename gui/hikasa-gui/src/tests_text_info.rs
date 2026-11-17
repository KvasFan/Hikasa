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
    f.app.0.on_watch(vec![WatchEvent::Inputs(vec![(
        spec.clone(),
        ChangeKind::Changed,
    )])]);
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
    f.app.0.on_watch(vec![WatchEvent::Inputs(vec![(
        spec.clone(),
        ChangeKind::Changed,
    )])]);
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
    f.app
        .0
        .on_watch(vec![WatchEvent::Inputs(vec![(right, ChangeKind::Changed)])]);
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
    f.app
        .0
        .on_watch(vec![WatchEvent::Inputs(vec![(spec, ChangeKind::Changed)])]);
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
