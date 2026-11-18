//! M2 W13b acceptance A13.22 (D-370): the Picture and Format tabs — each field writes the key `session edit --set`
//! writes; P-TABS-ID (open and OK without edits writes nothing, random edits write only their keys); refused words keep
//! OK off with core's message; OK relives the view without decoding again; the view state round trip (`[view]` keys,
//! the new `zoom`, `center` and `difference_pane` included) through a saved session and a reopen; a pre-W13 session
//! without the new keys loads with their defaults.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use hikasa_picture::{Orientation, Pane};
use hikasa_session::api::picture::PictureRulesFile;
use hikasa_session::api::{SessionEdit, SessionTarget};
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType};
use proptest::prelude::*;

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::picture::PictureLayout;
use crate::picture::viewport::{PaneRect, Zoom};
use crate::picture_tabs::{PictureFields, PictureFormatTab, PictureTab};
use crate::session_dialog::SessionTab;
use crate::tests::World;
use crate::tests_picture_open::{open, pair, picture_model};

fn picture_rules(file: PictureRulesFile) -> SessionRules {
    let mut session = Session::builtin(SessionType::PictureCompare);
    if let SessionRules::Picture(p) = &mut session.rules {
        p.picture = file;
    }
    session.rules
}

fn file_of(rules: &SessionRules) -> PictureRulesFile {
    match rules {
        SessionRules::Picture(p) => p.picture.clone(),
        other => panic!("{other:?}"),
    }
}

/// A13.22: each field of the Picture tab writes what `session edit --set rules.picture.<key>` writes.
#[test]
fn a13_22_each_field_writes_the_key_edit_writes() {
    let world = World::new();
    let svc = &world.service;
    let reference = SessionRef::parse("p/one").expect("ref");
    svc.session_save(
        &reference,
        &Session::builtin(SessionType::PictureCompare),
        false,
    )
    .expect("save");
    type Case = (&'static str, &'static str, fn(&mut PictureFields));
    let cases: &[Case] = &[
        ("rules.picture.mode", "\"range\"", |f| {
            f.mode = "range".into()
        }),
        ("rules.picture.tolerance", "17", |f| {
            f.tolerance = "17".into()
        }),
        ("rules.picture.blend", "30", |f| f.blend = "30".into()),
        ("rules.picture.ignore_unimportant", "true", |f| {
            f.ignore_unimportant = true
        }),
        ("rules.picture.auto_scale", "true", |f| f.auto_scale = true),
        ("rules.picture.tone_map", "\"reinhard\"", |f| {
            f.tone_map = "reinhard".into()
        }),
        ("rules.picture.max_megapixels", "64", |f| {
            f.max_megapixels = "64".into()
        }),
    ];
    for (key, value, edit) in cases {
        let current = svc.session_load(&reference).expect("load").value;
        let mut tab = PictureTab::default();
        tab.load(&current.rules);
        edit(&mut tab.fields);
        tab.check(svc, &current.rules);
        assert_eq!(tab.error(), None, "{key}");
        let written = tab.rules(&current.rules).expect("changed");
        svc.session_edit(
            &SessionTarget::Named(reference.clone()),
            &SessionEdit {
                set: vec![((*key).to_owned(), (*value).to_owned())],
                ..SessionEdit::default()
            },
            false,
        )
        .expect("edit");
        let edited = svc.session_load(&reference).expect("load").value;
        assert_eq!(file_of(&written), file_of(&edited.rules), "{key}");
        // Back to the defaults for the next row.
        svc.session_save(
            &reference,
            &Session::builtin(SessionType::PictureCompare),
            false,
        )
        .expect("save");
    }
}

/// A13.22: refused words are core's messages and keep OK off; the Format tab refuses a format of another kind.
#[test]
fn a13_22_refused_words() {
    let world = World::new();
    let svc = &world.service;
    let current = picture_rules(PictureRulesFile::default());
    for (edit, word) in [
        (
            (|f: &mut PictureFields| f.tolerance = "300".into()) as fn(&mut PictureFields),
            "300",
        ),
        (|f| f.blend = "101".into(), "101"),
        (|f| f.mode = "loud".into(), "loud"),
        (|f| f.tone_map = "hdr".into(), "hdr"),
        (|f| f.max_megapixels = "0".into(), "0"),
    ] {
        let mut tab = PictureTab::default();
        tab.load(&current);
        edit(&mut tab.fields);
        tab.check(svc, &current);
        assert!(
            tab.error().is_some_and(|e| e.contains(word)),
            "{word}: {:?}",
            tab.error()
        );
        assert_eq!(tab.rules(&current), None, "{word}: nothing written");
    }
    let mut format = PictureFormatTab::default();
    format.fields.left = "csv".into();
    format.check(svc, &current);
    assert!(
        format.error().is_some_and(|e| e.contains("picture")),
        "{:?}",
        format.error()
    );
    format.fields.left = String::new();
    format.check(svc, &current);
    assert_eq!(format.error(), None);
}

fn fields() -> impl Strategy<Value = Vec<(usize, String)>> {
    proptest::collection::vec(
        (0usize..7).prop_flat_map(|k| {
            let value = match k {
                0 => prop_oneof![Just("tolerance"), Just("range"), Just("blend")]
                    .prop_map(str::to_owned)
                    .boxed(),
                1 => (0u32..256).prop_map(|n| n.to_string()).boxed(),
                2 => (0u32..101).prop_map(|n| n.to_string()).boxed(),
                5 => prop_oneof![Just("clamp"), Just("reinhard")]
                    .prop_map(str::to_owned)
                    .boxed(),
                6 => (1u32..4097).prop_map(|n| n.to_string()).boxed(),
                _ => any::<bool>().prop_map(|b| b.to_string()).boxed(),
            };
            value.prop_map(move |v| (k, v))
        }),
        0..5,
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("HIKASA_HEAVY").map_or(128, |_| 10_000),
        ..ProptestConfig::default()
    })]

    /// P-TABS-ID for the Picture tab: open and OK without edits writes nothing; random edits change exactly their keys.
    #[test]
    fn p_tabs_id_picture(start_tol in 0u64..256, edits in fields()) {
        let world = World::new();
        let base = PictureRulesFile { tolerance: start_tol, ..PictureRulesFile::default() };
        let current = picture_rules(base.clone());
        let mut tab = PictureTab::default();
        tab.load(&current);
        prop_assert_eq!(tab.rules(&current), None, "no edit, nothing written");
        let mut want = base.clone();
        for (k, v) in &edits {
            let f = &mut tab.fields;
            match k {
                0 => { f.mode = v.clone(); want.mode = v.clone(); }
                1 => { f.tolerance = v.clone(); want.tolerance = v.parse().expect("n"); }
                2 => { f.blend = v.clone(); want.blend = v.parse().expect("n"); }
                3 => { f.ignore_unimportant = v == "true"; want.ignore_unimportant = v == "true"; }
                4 => { f.auto_scale = v == "true"; want.auto_scale = v == "true"; }
                5 => { f.tone_map = v.clone(); want.tone_map = v.clone(); }
                _ => { f.max_megapixels = v.clone(); want.max_megapixels = v.parse().expect("n"); }
            }
        }
        tab.check(&world.service, &current);
        prop_assert_eq!(tab.error(), None);
        match tab.rules(&current) {
            None => prop_assert_eq!(&want, &base, "unchanged fields write nothing"),
            Some(written) => prop_assert_eq!(file_of(&written), want),
        }
    }
}

/// A13.22: Rules opens the Session Settings on the Picture tab; OK with a new tolerance relives the view by
/// §1.13.9's table (an options job, no decode); the session is dirty.
#[test]
fn a13_22_ok_relives_the_view() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open(&world, &mut app, pair(&world));
    let loaded = Arc::clone(picture_model(&mut app, view).loaded().expect("loaded"));
    app.execute(CommandId::Rules);
    world.settle(&mut app);
    let w = app.win();
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::session_dialog::SessionSettingsDialog>()
        })
        .expect("settings");
    let titles: Vec<String> = d.tabs.iter().map(|t| t.title()).collect();
    assert_eq!(titles, ["Specs", "Picture", "Format"]);
    assert_eq!(d.tabs[d.active].title(), "Picture");
    d.tab_mut::<PictureTab>().expect("picture").fields.tolerance = "6".into();
    let svc = world.service.clone();
    d.poll(&svc);
    assert!(d.tab_error().is_none(), "{:?}", d.tab_error());
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    world.settle(&mut app);
    let m = picture_model(&mut app, view);
    assert_eq!(m.options.tolerance, 6);
    assert_eq!(m.comparison().expect("cmp").options().tolerance, 6);
    assert!(
        Arc::ptr_eq(m.loaded().expect("loaded"), &loaded),
        "not decoded again"
    );
    let _ = Ordering::SeqCst;
}

/// A13.22: the view state round trip through a saved session and a reopen: orientations, offset, checkerboard, layout,
/// zoom, centre and the difference pane; Swap Sides negates the offset in the session; a session written before W13
/// opens with the new keys' defaults.
#[test]
fn a13_22_view_state_round_trip() {
    let world = World::new();
    let mut app = world.app(None);
    let (l, r) = pair(&world);
    let view = open(&world, &mut app, (l.clone(), r.clone()));
    {
        let m = picture_model(&mut app, view);
        m.arrange(PaneRect::new(0.0, 0.0, 1200.0, 800.0), 0.0);
        m.focus = Pane::Left;
        m.execute(&world.service, CommandId::RotateRight);
        m.nudge(&world.service, 3, -1, false);
        m.checkerboard = false;
        m.layout = PictureLayout::Stacked;
        m.shown[2] = false;
        m.zoom_to(crate::picture::ZoomChoice::Percent(200));
    }
    world.settle(&mut app);
    // After the geometry landed (its origin shift moves the centre with the left picture).
    picture_model(&mut app, view).viewport.center = (12.5, 7.0);
    let tab = app.tab(view).expect("tab");
    let capture = crate::app::AppCore::capture_for_test(tab).expect("capture");
    let (session, _) = capture.session(&world.service).expect("session");
    let v = &session.view;
    assert_eq!(
        v.get("orientation_left").and_then(|v| v.as_str()),
        Some("cw")
    );
    assert_eq!(
        v.get("offset").map(ToString::to_string).as_deref(),
        Some("[3, -1]")
    );
    assert_eq!(v.get("checkerboard").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(v.get("layout").and_then(|v| v.as_str()), Some("stacked"));
    assert_eq!(v.get("zoom").and_then(|v| v.as_str()), Some("200%"));
    assert_eq!(
        v.get("difference_pane").and_then(|v| v.as_bool()),
        Some(false)
    );
    let reference = SessionRef::parse("pics/state").expect("ref");
    world
        .service
        .session_save(&reference, &session, false)
        .expect("save");
    let target = crate::app::OpenTarget::Window(app.win());
    app.0
        .open_named(reference.clone(), crate::app::OpenHow::Target(target));
    world.settle(&mut app);
    let again = app.active_tab().expect("tab").id;
    assert_ne!(again, view);
    let m = picture_model(&mut app, again);
    assert_eq!(m.options.geometry.orientation.left, Orientation::Cw90);
    assert_eq!(m.options.geometry.offset, (3, -1));
    assert!(!m.checkerboard);
    assert_eq!(m.layout, PictureLayout::Stacked);
    assert!(!m.shown[2]);
    assert_eq!(m.viewport.zoom, Zoom::Scale(2.0));
    assert_eq!(m.viewport.center, (12.5, 7.0));
    // Swap Sides of the saved session negates the offset.
    let mut swapped = world.service.session_load(&reference).expect("load").value;
    swapped.swap_sides();
    assert_eq!(
        swapped
            .view
            .get("offset")
            .map(ToString::to_string)
            .as_deref(),
        Some("[-3, 1]")
    );
    // A session written before W13: no zoom, centre or difference pane.
    let old = world.write(
        "config/sessions/pics/old.toml",
        &format!(
            "type = \"picture-compare\"\n[specs]\nleft = {:?}\nright = {:?}\n[view]\noffset = [1, 2]\n",
            l.display().to_string(),
            r.display().to_string()
        ),
    );
    let _ = old;
    app.0.open_named(
        SessionRef::parse("pics/old").expect("ref"),
        crate::app::OpenHow::Target(crate::app::OpenTarget::Window(app.win())),
    );
    world.settle(&mut app);
    let older = app.active_tab().expect("tab").id;
    let m = picture_model(&mut app, older);
    assert_eq!(m.viewport.zoom, Zoom::Fit);
    assert!(m.shown[2]);
    assert_eq!(m.options.geometry.offset, (1, 2));
}

/// A13.27: F1 in the view shows its topic.
#[test]
fn a13_27_help_topics_of_the_view() {
    assert_eq!(
        crate::commands::Scope::Picture.help_topic(),
        "view-picture-compare"
    );
    assert!(hikasa_session::help::topic("dialog-picture-info").is_some());
    let d = PictureTab::default();
    assert_eq!(d.help_topic(), "dialog-session-settings");
}
