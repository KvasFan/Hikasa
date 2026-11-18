//! A11.12 (D-305): the layouts and display options of a text view: the panes' shapes side by side, over under (the copy
//! buttons in the gutters), zoomed, one pane, with the splitter and `right_gutter_for_left_editor`; each command's
//! effect and check state; the whitespace marks per character class; the options of OPT-014/OPT-017 reaching the view;
//! the layout keys written on close and read back (a session's `[view]` first, an unknown word a warning); the
//! Display Font dialog.

use hikasa_core::Side;
use hikasa_session::settings::state::LayoutValue;
use hikasa_text::LineEnding;

use crate::commands::CommandId;
use crate::tests::World;
use crate::tests_text::Fixture;
use crate::text::layout::{
    DIVIDER_H, Details, Display, FindShown, STRIP_W, Tweaks, blank_mark, eol_mark, pane_geometry,
};

const L: &str = "int a = 1;\n\tint b = 2;\nint c = 3;\n";
const R: &str = "int a = 1;\n\tint b = 20;\nint c = 3;\n";

#[test]
fn a11_12_the_panes_shapes_per_layout() {
    let t = Tweaks::default();
    let d = Display::default();
    // Side by side: two panes and the strip, 50 %.
    let g = pane_geometry(&d, &t, false, 0, 1000.0, 600.0);
    let w = ((1000.0 - STRIP_W) * 0.5_f32).floor();
    assert_eq!(
        g.panes,
        vec![
            (0, [0.0, 0.0, w, 600.0]),
            (1, [w + STRIP_W, 0.0, 1000.0 - STRIP_W - w, 600.0])
        ]
    );
    assert_eq!(g.strip, Some([w, 0.0, STRIP_W, 600.0]));
    assert!(!g.buttons_in_gutter);
    assert_eq!(g.gutter_right, [false, false]);
    // `right_gutter_for_left_editor`: the left pane's numbers on its right, next to the strip.
    let right = Tweaks {
        right_gutter_for_left_editor: true,
        ..Tweaks::default()
    };
    assert_eq!(
        pane_geometry(&d, &right, false, 0, 1000.0, 600.0).gutter_right,
        [true, false]
    );
    // Over under: stacked, no strip, the copy buttons in the gutters; the flag does not apply.
    let ou = Display {
        over_under: true,
        ..Display::default()
    };
    let g = pane_geometry(&ou, &right, false, 0, 1000.0, 600.0);
    let top = ((600.0 - DIVIDER_H) * 0.5_f32).floor();
    assert_eq!(
        g.panes,
        vec![
            (0, [0.0, 0.0, 1000.0, top]),
            (1, [0.0, top + DIVIDER_H, 1000.0, 600.0 - DIVIDER_H - top])
        ]
    );
    assert!(g.strip.is_none() && g.buttons_in_gutter);
    assert_eq!(g.gutter_right, [false, false]);
    // The splitter.
    let split = Display {
        split: 0.3,
        ..Display::default()
    };
    let g = pane_geometry(&split, &t, false, 0, 1000.0, 600.0);
    assert_eq!(g.panes[0].1[2], ((1000.0 - STRIP_W) * 0.3_f32).floor());
    // Zoom Pane: the focus pane alone; Text Edit: one pane.
    let zoom = Display {
        zoom: true,
        ..Display::default()
    };
    assert_eq!(
        pane_geometry(&zoom, &t, false, 1, 800.0, 400.0).panes,
        vec![(1, [0.0, 0.0, 800.0, 400.0])]
    );
    assert_eq!(
        pane_geometry(&d, &t, true, 0, 800.0, 400.0).panes,
        vec![(0, [0.0, 0.0, 800.0, 400.0])]
    );
}

#[test]
fn a11_12_each_command_changes_the_display() {
    let mut f = Fixture::new(L, R);
    let checked = |f: &mut Fixture, id| f.m().checked(id);
    assert_eq!(checked(&mut f, CommandId::SideBySide), Some(true));
    f.run(CommandId::OverUnder);
    assert!(f.m().display().over_under);
    assert_eq!(checked(&mut f, CommandId::OverUnder), Some(true));
    assert_eq!(checked(&mut f, CommandId::SideBySide), Some(false));
    f.run(CommandId::SideBySide);
    assert!(!f.m().display().over_under);
    for (id, get) in [
        (
            CommandId::ShowSyntaxHighlighting,
            (|d: &Display| d.syntax) as fn(&Display) -> bool,
        ),
        (CommandId::ShowWS, |d: &Display| d.whitespace),
        (CommandId::ZoomPane, |d: &Display| d.zoom),
        (CommandId::ShowRuler, |d: &Display| d.ruler),
        (CommandId::WordWrap, |d: &Display| d.word_wrap),
    ] {
        let before = get(f.m().display());
        f.run(id);
        assert_eq!(get(f.m().display()), !before, "{id:?}");
        assert_eq!(checked(&mut f, id), Some(!before), "{id:?}");
        f.run(id);
        assert_eq!(get(f.m().display()), before, "{id:?}");
    }
    // The details panel is a radio with none: a second click hides it.
    for (id, kind) in [
        (CommandId::ShowTextDetails, Details::Text),
        (CommandId::ShowHexDetails, Details::Hex),
        (CommandId::ShowAlignDetails, Details::Alignment),
    ] {
        f.run(id);
        assert_eq!(f.m().display().details, kind);
        assert_eq!(checked(&mut f, id), Some(true));
    }
    f.run(CommandId::ShowAlignDetails);
    assert_eq!(f.m().display().details, Details::None);
    // Widen Left/Right move the splitter 5 % toward the other pane, within 10–90 %.
    f.run(CommandId::ZoomLeft);
    assert!((f.m().display().split - 0.55).abs() < 1e-6);
    for _ in 0..20 {
        f.run(CommandId::ZoomLeft);
    }
    assert!((f.m().display().split - 0.9).abs() < 1e-6);
    for _ in 0..40 {
        f.run(CommandId::ZoomRight);
    }
    assert!((f.m().display().split - 0.1).abs() < 1e-6);
    // Find, Replace: the panel; Esc (close) hides it.
    f.run(CommandId::Find);
    assert_eq!(f.m().display().find_panel, FindShown::Find);
    f.run(CommandId::Replace);
    assert_eq!(f.m().display().find_panel, FindShown::Replace);
    crate::text::find::close(f.m());
    assert_eq!(f.m().display().find_panel, FindShown::None);
}

#[test]
fn a11_12_whitespace_marks_per_character_class() {
    assert_eq!(blank_mark(' '), Some("·"));
    assert_eq!(blank_mark('\t'), Some("→"));
    for c in ['\u{a0}', '\u{2003}', '\u{3000}'] {
        assert_eq!(blank_mark(c), Some("°"), "{c:?}");
    }
    for c in ['a', '.', '\u{200b}'] {
        assert_eq!(blank_mark(c), None, "{c:?}");
    }
    assert_eq!(eol_mark(LineEnding::Lf), Some("¶"));
    assert_eq!(eol_mark(LineEnding::Cr), Some("¤"));
    assert_eq!(eol_mark(LineEnding::CrLf), Some("¶·"));
    assert_eq!(eol_mark(LineEnding::None), None);
}

#[test]
fn a11_12_the_options_reach_the_view() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tweaks.editor_display]\nextra_line_spacing_px = 3\ncolumn_line_at = 80\n\
         dim_inactive_pane_percent = 40\ncrosshatch_beyond_eof = false\nuse_orphan_color = false\n\
         right_gutter_for_left_editor = true\nsyntax_on_difference_lines = false\n[tweaks.interface]\n\
         single_vertical_scrollbar = true\nsticky_splitter_position = false\n",
    )]);
    let mut f = Fixture::in_world(world, L, R);
    let t = *f.m().tweaks();
    assert_eq!(t.extra_line_spacing_px, 3);
    assert_eq!(t.column_line_at, 80);
    assert_eq!(t.dim_inactive_pane_percent, 40);
    assert!(!t.crosshatch_beyond_eof && !t.use_orphan_color && !t.syntax_on_difference_lines);
    assert!(
        t.right_gutter_for_left_editor
            && t.single_vertical_scrollbar
            && !t.sticky_splitter_position
    );
    // Without a sticky splitter the split is not a layout key (each view starts at 50 %).
    f.run(CommandId::ZoomLeft);
    let keys = f.m().layout_keys();
    assert!(keys.iter().all(|(k, _)| *k != "splitter"), "{keys:?}");
}

#[test]
fn a11_12_layout_keys_are_written_on_close_and_read_back() {
    let mut f = Fixture::new(L, R);
    for id in [
        CommandId::OverUnder,
        CommandId::ShowWS,
        CommandId::ShowRuler,
        CommandId::ShowHexDetails,
        CommandId::ShowSyntaxHighlighting,
        CommandId::ZoomLeft,
        CommandId::Find,
    ] {
        f.run(id);
    }
    f.m().display_mut().details_height = 200.0;
    f.m().display_mut().browse_mode = hikasa_text::nav::BrowseMode::Edit;
    f.run(CommandId::CloseTab);
    f.settle();
    let state = f.app.0.state().clone();
    let v = |key: &str| state.layout_value("text-compare", key);
    assert_eq!(v("layout"), Some(LayoutValue::Text("over_under".into())));
    assert_eq!(v("whitespace"), Some(LayoutValue::Bool(true)));
    assert_eq!(v("ruler"), Some(LayoutValue::Bool(true)));
    assert_eq!(v("syntax"), Some(LayoutValue::Bool(false)));
    assert_eq!(v("details"), Some(LayoutValue::Text("hex".into())));
    assert_eq!(v("details_height"), Some(LayoutValue::Float(200.0)));
    assert_eq!(v("find_panel"), Some(LayoutValue::Text("find".into())));
    assert_eq!(v("browse_mode"), Some(LayoutValue::Text("edit".into())));
    assert_eq!(v("splitter"), Some(LayoutValue::Float(0.55)));
    // Read back by the next view of the type.
    let l = f.paths[0].clone();
    let r = f.paths[1].clone();
    let view = f.world.open(&mut f.app.0, &l, &r);
    f.view = view;
    f.settle();
    let d = f.m().display().clone();
    assert!(d.over_under && d.whitespace && d.ruler && !d.syntax);
    assert_eq!(d.details, Details::Hex);
    assert_eq!(d.find_panel, FindShown::Find);
    assert_eq!(d.browse_mode, hikasa_text::nav::BrowseMode::Edit);
    assert!((d.split - 0.55).abs() < 1e-6);
    assert_eq!(d.details_height, 200.0);
    // A session's `[view]` wins over the type's layout, and an unknown word falls back with a warning.
    let mut state = state.clone();
    state.set_layout_value(
        "text-compare",
        "details",
        LayoutValue::Text("sideways".into()),
    );
    let mut fresh = Display::default();
    let warnings = fresh.read_layout(&state, "text-compare", false, true);
    assert_eq!(fresh.details, Details::None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("invalid_config_value"),
        "{warnings:?}"
    );
    let mut session = hikasa_session::sessions::OpaqueTable::new();
    session.insert(
        "layout".into(),
        hikasa_session::sessions::OpaqueTable::from_iter([("x".to_owned(), "side_by_side".into())])
            ["x"]
            .clone(),
    );
    state.overlay_view("text-compare", &session);
    let mut fresh = Display::default();
    fresh.read_layout(&state, "text-compare", false, true);
    assert!(!fresh.over_under, "the session's [view] first");
}

#[test]
fn a11_12_the_display_font_dialog() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::DisplayFont);
    let win = f.app.win();
    let service = f.world.service.clone();
    let step = {
        let w = f.app.window_mut(win).expect("window");
        let d = w
            .dialogs
            .find_mut::<crate::text::layout::DisplayFontDialog>()
            .expect("the Display Font dialog");
        assert_eq!(
            crate::dialog::DialogModel::help_topic(d),
            "dialog-display-font"
        );
        d.size = "99".into();
        assert_eq!(
            crate::dialog::DialogModel::on_button(d, crate::dialog::ButtonRole::Ok, &service),
            crate::dialog::DialogStep::Stay
        );
        assert!(d.error.is_some());
        d.size = "20".into();
        d.family = "Monospace".into();
        crate::dialog::DialogModel::on_button(d, crate::dialog::ButtonRole::Ok, &service)
    };
    assert_eq!(step, crate::dialog::DialogStep::Close);
    let closed = f
        .app
        .window_mut(win)
        .and_then(|w| w.dialogs.pop())
        .expect("dialog");
    f.app.0.dialog_closed_for_test(closed);
    assert_eq!(f.m().font_px(), 20);
    assert_eq!(f.m().display().font_family.as_deref(), Some("Monospace"));
    let keys = f.m().layout_keys();
    assert!(keys.contains(&("font_family", LayoutValue::Text("Monospace".into()))));
    assert!(keys.contains(&("font_size", LayoutValue::Float(20.0))));
    let _ = Side::Left;
}
