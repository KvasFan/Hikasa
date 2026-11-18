//! M2 W13b acceptance A13.14 (D-365) and A13.23 (D-371) for pictures: the status items from `probe` at corners,
//! outside the area and in a one-sided region; Info's figures and its Copy (core's `human_lines`); the headers' fields;
//! Copy of each pane in each mode (a `FixedClipboard` with images) equal to `picture_write_with`'s render with the
//! window's colours and the checkerboard flag; Open Clipboard with an image and with text; the context-menu and menu
//! goldens; every implemented `picture-compare` entry with its effect; F1 of the Info dialog.

use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_picture::{DiffMode, Pane};
use hikasa_session::api::ReportTarget;
use hikasa_session::clipboard::FixedClipboard;
use hikasa_session::commands::{CommandStatus, catalogue};

use crate::commands::{Area, CommandId, Scope, lookup};
use crate::dialog::DialogModel;
use crate::menus::{Menu, MenuContext, MenuTitle, menu_bar, render_text};
use crate::picture::NO_PICTURE;
use crate::picture::info::{COPY, PictureInfoDialog};
use crate::picture::viewport::PaneRect;
use crate::tests::{TestApp, World};
use crate::tests_picture_open::{base, open, pair, picture_model, png, rgba, wait};

const CONTENT: PaneRect = PaneRect::new(0.0, 0.0, 1200.0, 800.0);

/// A13.14: the status bar's probe is core's `probe_line` of the area pixel under the pointer, in any pane: corners,
/// one-sided regions; nothing outside the area. The other items: offset, zoom, mode, verdict.
#[test]
fn a13_14_status_items_from_the_probe() {
    let world = World::new();
    let l = png(&world, "st/a.png", 30, 20, base);
    let r = png(&world, "st/b.png", 12, 26, |x, y| base(x + 1, y));
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l, r));
    let m = picture_model(&mut app, view);
    m.arrange(CONTENT, 0.0);
    let c = Arc::clone(m.comparison().expect("cmp"));
    let area = m.area();
    let scale = m.scale();
    for slot in m.slots().to_vec() {
        // Corners of the area, a left-only pixel (x 20) and a right-only one (y 23).
        for (x, y) in [(0, 0), (29, 0), (0, 25), (29, 25), (20, 5), (3, 23)] {
            let p = m.viewport.to_screen(
                slot.picture,
                scale,
                area,
                (f64::from(x) + 0.5, f64::from(y) + 0.5),
            );
            m.pointer_moved(p);
            let probe = c.probe(x, y);
            assert_eq!(m.probe(), Some(&probe), "{:?} ({x}, {y})", slot.pane);
            let status = m.status_items(&world.service);
            assert_eq!(status.left, hikasa_report::probe_line(&probe));
        }
        // Outside the area (the pane's corner, Fit leaves a margin).
        m.pointer_moved((slot.picture.x + 1.0, slot.picture.y + 1.0));
        assert_eq!(m.probe(), None);
        assert_eq!(m.status_items(&world.service).left, "");
    }
    let status = m.status_items(&world.service);
    assert_eq!(status.right[0], "Offset 0, 0");
    assert!(status.right[1].starts_with("Fit "), "{:?}", status.right);
    assert_eq!(status.right[2], "Tolerance 0");
    assert_eq!(status.right[3], "Important differences");
    m.set_blend(40);
    assert_eq!(m.status_items(&world.service).right[2], "Blend 40 %");
}

/// A13.14: Info's figures are core's (counts and percentages with 4 decimals, the bounds), *Copy* places
/// `human_lines()` and the verdict, *Show* fits the view to the bounds; F1 shows its topic.
#[test]
fn a13_14_info_copy_and_show() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let clipboard = Arc::new(FixedClipboard::empty());
    app.set_script_clipboard(Arc::clone(&clipboard) as _);
    let view = open(&world, &mut app, (l, r));
    picture_model(&mut app, view).arrange(CONTENT, 0.0);
    app.execute(CommandId::Info);
    let w = app.win();
    let response = picture_model(&mut app, view)
        .response(&world.service)
        .expect("response");
    let dialog = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<PictureInfoDialog>())
        .expect("the Info dialog");
    assert_eq!(dialog.help_topic(), "dialog-picture-info");
    let mut lines = response.human_lines();
    lines.push("Important differences".to_owned());
    assert_eq!(dialog.text(), lines.join("\n") + "\n");
    let s = response.comparison.stats();
    let pixels = &dialog.sections()[0];
    assert_eq!(pixels.rows[0].values, [s.pixels.to_string()]);
    assert_eq!(
        pixels.rows[3].values,
        [format!("{} ({:.4} %)", s.different, s.percent(s.different))]
    );
    dialog.on_button(COPY, &world.service);
    assert_eq!(
        clipboard.payloads().last().map(|p| p.text.clone()),
        Some(dialog.text().to_owned())
    );
    // *Show* closes the dialog and the window fits the view to the bounds.
    app.0.dialog_button(w, crate::picture::info::SHOW);
    assert!(
        app.0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<PictureInfoDialog>())
            .is_none()
    );
    let m = picture_model(&mut app, view);
    let b = s.bounds.expect("bounds");
    let c = m.viewport.center;
    assert!((c.0 - (b.x as f64 + f64::from(b.width) / 2.0)).abs() < 1e-9);
    assert!(m.scale() > 1.0, "zoomed onto the differences");
}

/// A13.14: the File Info headers from `PictureInfo`; the difference pane's: the area, the mode and the verdict.
#[test]
fn a13_14_headers() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l.clone(), r));
    let m = picture_model(&mut app, view);
    let left = m.header_lines(Pane::Left, &world.service);
    assert!(left[0].ends_with("a.png"), "{left:?}");
    let size = std::fs::metadata(&l).expect("stat").len();
    assert_eq!(
        left[1],
        format!("40\u{d7}30  png  8 bits  rgba8  alpha  {size} bytes")
    );
    let diff = m.header_lines(Pane::Difference, &world.service);
    assert_eq!(
        diff,
        [
            "Difference 40\u{d7}30  Tolerance 0",
            "Important differences"
        ]
    );
}

/// A13.14: Copy of each pane in each mode with and without the checkerboard places what `picture_write_with` renders
/// with the window's colours (the dark scheme here); a message line names it. Mutation: Copy ignores the checkerboard.
#[test]
fn a13_14_copy_each_pane_in_each_mode() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let clipboard = Arc::new(FixedClipboard::empty());
    app.set_script_clipboard(Arc::clone(&clipboard) as _);
    let view = open(&world, &mut app, (l, r));
    // The window follows a dark OS theme (UI-023, live): Copy renders in the window's colours.
    app.0.set_os_dark(true);
    world.settle(&mut app);
    let dark = hikasa_session::settings::theme::Scheme::resolve("dark", None)
        .picture
        .to_render();
    assert_eq!(picture_model(&mut app, view).colors(), dark);
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        for mode in [
            DiffMode::Tolerance,
            DiffMode::Range,
            DiffMode::Blend { percent: 70 },
        ] {
            for checkerboard in [true, false] {
                let m = picture_model(&mut app, view);
                m.focus = pane;
                match mode {
                    DiffMode::Blend { percent } => m.set_blend(percent),
                    mode => m.set_mode(mode),
                }
                m.checkerboard = checkerboard;
                let c = Arc::clone(m.comparison().expect("cmp"));
                let before = clipboard.images().len();
                app.execute(CommandId::Copy);
                world.settle(&mut app);
                let placed = clipboard.images();
                assert_eq!(placed.len(), before + 1, "{pane:?} {mode:?}");
                let job = world
                    .service
                    .job(JobKind::PictureCompare, Arc::new(hikasa_core::NullProgress));
                let want = world
                    .service
                    .picture_write_with(
                        &c,
                        pane,
                        mode,
                        checkerboard,
                        &dark,
                        &ReportTarget::Clipboard,
                        &job,
                    )
                    .expect("render")
                    .clipboard
                    .expect("image");
                let got = placed.last().expect("image");
                assert_eq!((got.width, got.height), (want.width, want.height));
                assert!(
                    got.rgba == want.rgba,
                    "{pane:?} {mode:?} checkerboard {checkerboard}"
                );
            }
        }
    }
    let message = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("a line");
    assert!(
        message
            .text
            .contains("Copied the difference picture (40 × 30)"),
        "{}",
        message.text
    );
}

/// A13.14: Open Clipboard with an image reloads the current side from `clipboard://`; with text the message line.
#[test]
fn a13_14_open_clipboard() {
    let pixels = rgba(40, 30, base);
    let fixed = Arc::new(FixedClipboard::image(40, 30, pixels));
    let provider = Arc::clone(&fixed);
    let world = World::with_init(move |init| init.clipboard = provider);
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app, (l, r));
    picture_model(&mut app, view).focus = Pane::Difference;
    app.execute(CommandId::OpenClipboard);
    world.settle(&mut app);
    let m = picture_model(&mut app, view);
    assert_eq!(m.request().right.to_string_lossy(), "clipboard://");
    let response = m.response(&world.service).expect("response");
    assert_eq!(
        response.comparison.stats().different,
        0,
        "the clipboard holds the left picture"
    );
    // Text: the message.
    fixed.set_text("not a picture");
    picture_model(&mut app, view).focus = Pane::Left;
    app.execute(CommandId::OpenClipboard);
    world.settle(&mut app);
    let message = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("a line");
    assert_eq!(message.text, NO_PICTURE);
    assert!(
        !picture_model(&mut app, view)
            .request()
            .left
            .to_string_lossy()
            .contains("clipboard")
    );
}

fn menus_text(app: &TestApp) -> String {
    let w = app.win();
    let checked = |id: CommandId| app.0.checked(w, id);
    let mc = MenuContext {
        tables: app.tables(),
        scope: app.0.scope(w),
        ctx: app.0.ctx(w),
        checked: &checked,
        workspaces: &[],
        columns: &[],
        open_with: &[],
        formats: &[],
    };
    render_text(&menu_bar(&mc))
}

/// A13.23: the menu bar of a Picture Compare view and the context menus of its panes (goldens).
#[test]
fn a13_23_menu_and_context_menu_goldens() {
    let world = World::new();
    let (l, r) = pair(&world);
    let mut app = world.app(None);
    open(&world, &mut app, (l, r));
    crate::tests_shell::golden("menus-picture-compare.txt", &menus_text(&app));
    let w = app.win();
    let mut text = String::new();
    for pane in [Pane::Left, Pane::Difference] {
        text.push_str(&format!("## {pane:?} pane\n"));
        text.push_str(&render_text(&[Menu {
            title: MenuTitle::Edit,
            items: app.0.pane_menu(w, Area::PicturePane(pane)),
        }]));
    }
    crate::tests_shell::golden("menus/picture-pane.txt", &text);
    // No shortcut conflict within the view (the zoom keys and the display font keys live in different scopes).
    let tables = app.tables();
    let mut seen = std::collections::HashMap::new();
    for row in crate::commands::rows_for(Scope::Picture) {
        if !crate::commands::shown(tables, Scope::Picture, row.id)
            || row.enabled_when == crate::commands::Predicate::Never
        {
            continue;
        }
        if let Some(entry) = tables.entry(Scope::Picture, row.id) {
            for s in &entry.shortcuts {
                if let Some(other) = seen.insert(s.to_string(), row.id) {
                    assert_eq!(other, row.id, "{s} twice");
                }
            }
        }
    }
}

/// What a command must have done on a fresh view.
type Effect = fn(&World, &mut TestApp, hikasa_session::ipc::ViewId);

/// A13.23: every implemented entry of the `picture-compare` section has a Picture row and its effect on a fresh view.
#[test]
fn a13_23_every_picture_entry_has_its_effect() {
    let section = catalogue().section("picture-compare").expect("section");
    for entry in section
        .commands
        .iter()
        .filter(|e| e.status == CommandStatus::Implemented)
    {
        let id = CommandId::from_name(&entry.id).expect("known");
        assert!(
            lookup(Scope::Picture, id).is_some_and(|r| r.scope == Scope::Picture),
            "{id:?} is implemented but has no Picture row"
        );
    }
    let effects: &[(CommandId, Effect)] = &[
        (CommandId::DiffTolerance, |_, app, v| {
            assert_eq!(picture_model(app, v).mode, DiffMode::Tolerance);
            assert_eq!(app.checked(CommandId::DiffTolerance), Some(true));
        }),
        (CommandId::DiffRange, |_, app, v| {
            assert_eq!(picture_model(app, v).mode, DiffMode::Range);
        }),
        (CommandId::DiffBlend, |_, app, v| {
            assert!(matches!(picture_model(app, v).mode, DiffMode::Blend { .. }));
        }),
        (CommandId::AutoScale, |_, app, v| {
            assert!(picture_model(app, v).options.geometry.auto_scale);
        }),
        (CommandId::RotateRight, |_, app, v| {
            assert_eq!(
                picture_model(app, v).options.geometry.orientation.left,
                hikasa_picture::Orientation::Cw90
            );
        }),
        (CommandId::RotateLeft, |_, app, v| {
            assert_eq!(
                picture_model(app, v).options.geometry.orientation.left,
                hikasa_picture::Orientation::Ccw90
            );
        }),
        (CommandId::FlipHorz, |_, app, v| {
            assert_eq!(
                picture_model(app, v).options.geometry.orientation.left,
                hikasa_picture::Orientation::FlipH
            );
        }),
        (CommandId::FlipVert, |_, app, v| {
            assert_eq!(
                picture_model(app, v).options.geometry.orientation.left,
                hikasa_picture::Orientation::FlipV
            );
        }),
        (CommandId::OffsetReset, |_, app, v| {
            assert_eq!(picture_model(app, v).options.geometry.offset, (0, 0));
        }),
        (CommandId::BlendToggle, |_, app, v| {
            assert_eq!(picture_model(app, v).blend_toggle, Some(0));
        }),
        (CommandId::FullScreen, |_, app, v| {
            assert!(picture_model(app, v).full_screen.is_some());
        }),
        (CommandId::ZoomList, |_, app, v| {
            assert_eq!(
                picture_model(app, v).viewport.zoom,
                crate::picture::viewport::Zoom::Fit
            );
        }),
        (CommandId::ZoomIn, |_, app, v| {
            assert!(matches!(
                picture_model(app, v).viewport.zoom,
                crate::picture::viewport::Zoom::Scale(_)
            ));
        }),
        (CommandId::ZoomOut, |_, app, v| {
            assert!(matches!(
                picture_model(app, v).viewport.zoom,
                crate::picture::viewport::Zoom::Scale(_)
            ));
        }),
        (CommandId::ZoomFit, |_, app, v| {
            assert_eq!(
                picture_model(app, v).viewport.zoom,
                crate::picture::viewport::Zoom::Fit
            );
        }),
        (CommandId::ZoomActual, |_, app, v| {
            assert_eq!(picture_model(app, v).scale(), 1.0);
        }),
        (CommandId::Checkerboard, |_, app, v| {
            assert!(!picture_model(app, v).checkerboard);
        }),
        (CommandId::ShowDifferencePane, |_, app, v| {
            assert!(!picture_model(app, v).shown[2]);
        }),
        (CommandId::Tolerance, |_, app, v| {
            assert_eq!(
                picture_model(app, v).options.tolerance,
                0,
                "no argument: unchanged"
            );
        }),
        (CommandId::BlendPercent, |_, _, _| {}),
        // M2 W13c (D-366): the report dialog (the picture-compare section's Report row).
        (CommandId::Report, |_, app, _| {
            assert!(
                crate::tests_sync_view::find::<crate::report_dialog::ReportDialog>(app).is_some()
            );
        }),
    ];
    for entry in section
        .commands
        .iter()
        .filter(|e| e.status == CommandStatus::Implemented)
    {
        let id = CommandId::from_name(&entry.id).expect("known");
        assert!(
            effects.iter().any(|(e, _)| *e == id),
            "{id:?} has no effect in this test"
        );
    }
    let world = World::new();
    let (l, r) = pair(&world);
    for &(id, check) in effects {
        let mut app = world.app(None);
        let view = open(&world, &mut app, (l.clone(), r.clone()));
        picture_model(&mut app, view).arrange(CONTENT, 0.0);
        picture_model(&mut app, view).focus = Pane::Left;
        if id == CommandId::OffsetReset {
            picture_model(&mut app, view).nudge(&world.service, 2, 2, false);
            world.settle(&mut app);
        }
        if id == CommandId::BlendPercent {
            picture_model(&mut app, view).set_mode(DiffMode::Blend { percent: 50 });
        }
        assert!(app.enabled(id), "{id:?} is available");
        app.execute(id);
        world.settle(&mut app);
        check(&world, &mut app, view);
        let m = picture_model(&mut app, view);
        wait(m, &world);
    }
}
