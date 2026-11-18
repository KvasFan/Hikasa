//! W13b review fixes: the picture view through the real drawing code with a headless context (scroll bars, the
//! window budget across tabs, the offset drag through the window, Esc, a dialog over the view, and the later fixes).

use egui::{Event, PointerButton, pos2};

use super::HikasaApp;
use super::tests::{frame, new_app, pointer_button, raw};
use crate::picture::PictureViewModel;
use crate::picture::tiles::TEXTURE_BUDGET;
use crate::picture::viewport::Zoom;
use crate::tests::World;
use crate::tests_picture_open::{base, pair, png};
use crate::window::{TabContent, ViewModel};
use hikasa_session::ipc::ViewId;

struct Screen {
    world: World,
    ctx: egui::Context,
    app: HikasaApp,
    time: f64,
}

impl Screen {
    fn new() -> Screen {
        let world = World::new();
        let ctx = egui::Context::default();
        let app = new_app(&world, &ctx);
        Screen {
            world,
            ctx,
            app,
            time: 0.0,
        }
    }

    fn input(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let mut input = raw(self.time);
        input.events = events;
        frame(&mut self.app, &self.ctx, input);
    }

    fn settle(&mut self) {
        self.world.settle(&mut self.app.core);
        for _ in 0..3 {
            self.input(Vec::new());
        }
    }

    fn model(&mut self, view: ViewId) -> &mut PictureViewModel {
        match self.app.core.tab_mut(view).map(|t| &mut t.content) {
            Some(TabContent::Live(ViewModel::Picture(m))) => m,
            _ => panic!("not a picture view"),
        }
    }
}

/// A drag of a scroll bar's knob by `by` points along the bar (axis 0 horizontal, 1 vertical) in `pane` at 4,800 %:
/// the offset and the centre after it, and the centre before.
fn bar_drag(
    pane: hikasa_picture::Pane,
    axis: usize,
    by: f32,
) -> ((i32, i32), (f64, f64), (f64, f64)) {
    let mut s = Screen::new();
    let (l, r) = pair(&s.world);
    let view = s.world.open(&mut s.app.core, &l, &r);
    s.settle();
    {
        let m = s.model(view);
        m.viewport.zoom = Zoom::Scale(48.0);
        m.viewport.center = (20.0, 15.0);
    }
    s.settle();
    let (rect, bar, before) = {
        let m = s.model(view);
        let rect = m
            .slots()
            .iter()
            .find(|x| x.pane == pane)
            .expect("the pane")
            .picture;
        let bar = m.scrollbars(pane)[axis].expect("a bar at 4,800 %");
        (rect, bar, m.viewport.center)
    };
    // The knob's middle (the bars are 9 points thick at the pane's bottom and right edges).
    let from = if axis == 0 {
        let track = rect.w - 9.0;
        pos2(
            (rect.x + track * (bar.0 + bar.1 / 2.0)) as f32,
            (rect.y + rect.h - 4.5) as f32,
        )
    } else {
        let track = rect.h - 9.0;
        pos2(
            (rect.x + rect.w - 4.5) as f32,
            (rect.y + track * (bar.0 + bar.1 / 2.0)) as f32,
        )
    };
    let along = |d: f32| {
        if axis == 0 {
            pos2(from.x + d, from.y)
        } else {
            pos2(from.x, from.y + d)
        }
    };
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    for step in 1..=4 {
        s.input(vec![Event::PointerMoved(along(by * step as f32 / 4.0))]);
    }
    s.input(vec![pointer_button(
        along(by),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    let m = s.model(view);
    (m.options.geometry.offset, m.viewport.center, before)
}

/// F4: a drag on a pane's scroll bar scrolls every pane in the bar's direction; it never starts the pane's own drag
/// (in the difference pane the pane's `interact`, registered after the bars, took the press and moved the offset; in
/// the left and right panes it panned the other way). Every pane, both bars, both directions.
#[test]
fn the_scroll_bar_of_the_difference_pane_scrolls() {
    use hikasa_picture::Pane;
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        for axis in [0, 1] {
            for by in [-60.0f32, 60.0] {
                let (offset, after, before) = bar_drag(pane, axis, by);
                let moved = if axis == 0 {
                    after.0 - before.0
                } else {
                    after.1 - before.1
                };
                let still = if axis == 0 {
                    after.1 - before.1
                } else {
                    after.0 - before.0
                };
                assert_eq!(offset, (0, 0), "{pane:?} axis {axis} by {by}: the offset");
                assert!(
                    moved * f64::from(by) > 0.0 && still.abs() < 1e-9,
                    "{pane:?} axis {axis} by {by}: the centre {before:?} → {after:?}"
                );
            }
        }
    }
}

/// The resident bytes of every picture view of the window.
fn resident(s: &mut Screen, views: &[ViewId]) -> u64 {
    views.iter().map(|v| s.model(*v).tiles.bytes()).sum()
}

/// Opens `tabs` picture tabs of a `w` × `h` pair one after the other: each draws completely when shown and the window's
/// resident texels stay ≤ 64 MiB at every frame (§1.13.11). The hidden tabs' overviews give way, least recently shown
/// first; then the first tabs are shown again and draw again.
fn budget_across_tabs(w: u32, h: u32, tabs: usize) {
    let mut s = Screen::new();
    let l = png(&s.world, "big/a.png", w, h, base);
    let r = png(&s.world, "big/b.png", w, h, |x, y| {
        let mut p = base(x, y);
        if x % 97 == 0 {
            p[0] ^= 0x40;
        }
        p
    });
    let mut views = Vec::new();
    let mut drawn = Vec::new();
    let mut most = 0u64;
    for _ in 0..tabs {
        let view = s.world.open(&mut s.app.core, &l, &r);
        views.push(view);
        s.settle();
        let mut done = false;
        for _ in 0..2000 {
            s.input(Vec::new());
            most = most.max(resident(&mut s, &views));
            let m = s.model(view);
            if !m.tiles.wanted_is_empty() && m.tiles.complete() && !m.tiles.busy() {
                done = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drawn.push(done);
    }
    // The first tabs again (their overviews were taken): each draws again.
    for k in 0..2 {
        let window = s.app.core.root();
        if let Some(w) = s.app.core.window_mut(window) {
            w.active = k;
        }
        let view = views[k];
        let mut done = false;
        for _ in 0..2000 {
            s.input(Vec::new());
            most = most.max(resident(&mut s, &views));
            let m = s.model(view);
            if !m.tiles.wanted_is_empty() && m.tiles.complete() && !m.tiles.busy() {
                done = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drawn.push(done);
    }
    assert!(
        most <= TEXTURE_BUDGET && drawn.iter().all(|d| *d),
        "{:.1} MiB resident at most in one window (budget 64 MiB); each shown tab fully drawn: {drawn:?}",
        most as f64 / f64::from(1 << 20)
    );
}

/// F1: hidden tabs keep their overviews (12 MiB for a 1,024 × 1,024 pair) and the shown view got the budget less
/// theirs: from the sixth tab on it could not upload its overviews and stayed undrawn.
#[test]
fn the_budget_holds_across_picture_tabs() {
    budget_across_tabs(1024, 1024, 8);
}

/// F1 with 12 MP pairs (9 MiB of overviews a tab: the eighth tab starved). Decoding and comparing eight 12 MP pairs
/// takes minutes in a debug build: it runs with the heavy proptests (`HIKASA_HEAVY=1`); the regular run takes a 3 MP
/// pair with the same overview size (1,000 × 750 at level 1 instead of level 2).
#[test]
fn the_budget_holds_across_picture_tabs_of_12_mp() {
    if std::env::var_os("HIKASA_HEAVY").is_some() {
        budget_across_tabs(4000, 3000, 8);
    } else {
        budget_across_tabs(2000, 1500, 8);
    }
}

fn esc() -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A picture view of the 40 × 30 pair at 100 % centred, after its first frames; the difference pane's centre.
fn at_actual_size(s: &mut Screen) -> (ViewId, egui::Pos2) {
    let (l, r) = pair(&s.world);
    let view = s.world.open(&mut s.app.core, &l, &r);
    s.settle();
    {
        let m = s.model(view);
        m.viewport.zoom = Zoom::Scale(1.0);
        m.viewport.center = (20.0, 15.0);
    }
    s.settle();
    let d = s
        .model(view)
        .slots()
        .iter()
        .find(|x| x.pane == hikasa_picture::Pane::Difference)
        .expect("difference pane")
        .picture;
    (
        view,
        pos2((d.x + d.w / 2.0) as f32, (d.y + d.h / 2.0) as f32),
    )
}

/// F9: an offset drag through the window moves the offset by the whole drag (Δpoints × ppp / z, §1.13.12). The pane
/// calls `press` again when egui reports `drag_started`, which drops the motion of that frame.
#[test]
fn a_window_drag_moves_the_offset_by_its_whole_distance() {
    let mut s = Screen::new();
    let (view, from) = at_actual_size(&mut s);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    for step in 1..=4 {
        s.input(vec![Event::PointerMoved(pos2(
            from.x + 20.0 * step as f32,
            from.y,
        ))]);
    }
    s.input(vec![pointer_button(
        pos2(from.x + 80.0, from.y),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    assert_eq!(
        s.model(view).options.geometry.offset,
        (80, 0),
        "80 points at 100 %"
    );
}

/// F9: Esc during a window drag returns to the offset at the drag's start, even when the first moves were below egui's
/// drag threshold (the second `press` at `drag_started` takes the offset of that moment as the start).
#[test]
fn esc_returns_to_the_offset_at_the_drag_start() {
    let mut s = Screen::new();
    let (view, from) = at_actual_size(&mut s);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    for dx in [3.0, 5.0, 12.0, 30.0] {
        s.input(vec![Event::PointerMoved(pos2(from.x + dx, from.y))]);
    }
    assert_ne!(s.model(view).options.geometry.offset, (0, 0));
    s.input(vec![esc()]);
    s.input(vec![pointer_button(
        pos2(from.x + 30.0, from.y),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    assert_eq!(
        s.model(view).options.geometry.offset,
        (0, 0),
        "Esc returns to the start"
    );
}

/// F10: after Esc the drag is over while the button stays down: moving on does not start a new drag.
#[test]
fn moving_after_esc_does_not_restart_the_drag() {
    let mut s = Screen::new();
    let (view, from) = at_actual_size(&mut s);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    s.input(vec![Event::PointerMoved(pos2(from.x + 20.0, from.y))]);
    s.input(vec![Event::PointerMoved(pos2(from.x + 40.0, from.y))]);
    s.input(vec![esc()]);
    s.input(vec![Event::PointerMoved(pos2(from.x + 60.0, from.y))]);
    s.input(vec![Event::PointerMoved(pos2(from.x + 80.0, from.y))]);
    s.input(vec![pointer_button(
        pos2(from.x + 80.0, from.y),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    assert_eq!(
        s.model(view).options.geometry.offset,
        (0, 0),
        "no drag after Esc"
    );
}

/// F11: a dialog takes the input: the wheel over the picture view under it neither pans nor zooms, and Esc closes the
/// dialog without leaving full screen (as `a_text_view_under_a_dialog_does_not_scroll`).
#[test]
fn the_picture_view_under_a_dialog_keeps_still() {
    let mut s = Screen::new();
    let (view, at) = at_actual_size(&mut s);
    let window = s.app.core.root();
    s.app
        .core
        .execute(window, crate::commands::CommandId::FullScreen);
    s.settle();
    assert!(s.model(view).full_screen.is_some());
    s.app
        .core
        .execute(window, crate::commands::CommandId::About);
    s.settle();
    assert!(s.app.core.ctx(window).dialog_open);
    let before = s.model(view).viewport;
    s.input(vec![Event::PointerMoved(at)]);
    s.input(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -3.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    }]);
    // A pinch and a primary drag over the difference pane under the dialog.
    s.input(vec![Event::Zoom(1.5)]);
    s.input(vec![pointer_button(at, PointerButton::Primary, true)]);
    for dx in [10.0, 20.0, 40.0] {
        s.input(vec![Event::PointerMoved(pos2(at.x + dx, at.y))]);
    }
    s.input(vec![pointer_button(
        pos2(at.x + 40.0, at.y),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![esc()]);
    s.settle();
    let m = s.model(view);
    let got = (
        m.viewport == before,
        m.full_screen.is_some(),
        m.options.geometry.offset,
    );
    assert_eq!(
        got,
        (true, true, (0, 0)),
        "(view unchanged, still full screen, offset)"
    );
}

/// A press at `from`, a drag to `to` in four moves, the release.
fn drag(s: &mut Screen, from: egui::Pos2, to: egui::Pos2) {
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    for k in 1..=4 {
        let t = k as f32 / 4.0;
        s.input(vec![Event::PointerMoved(from + (to - from) * t)]);
    }
    s.input(vec![pointer_button(to, PointerButton::Primary, false)]);
    s.input(vec![Event::PointerGone]);
    s.settle();
}

/// Finding 14: both splitters drag in both layouts (the left and right panes', the difference pane's); the offset
/// stays; the positions go to `[layout.picture-compare]` when the view closes and the next view opens with them.
#[test]
fn the_splitters_drag_and_persist() {
    use crate::picture::{PictureLayout, Split};
    use hikasa_session::settings::state::LayoutValue;
    let mut s = Screen::new();
    let (l, r) = pair(&s.world);
    for layout in [PictureLayout::SideBySide, PictureLayout::Stacked] {
        let view = s.world.open(&mut s.app.core, &l, &r);
        s.settle();
        s.model(view).layout = layout;
        s.settle();
        for split in [Split::Panes, Split::Difference] {
            let (line, vertical, before) = {
                let m = s.model(view);
                let (_, line, vertical) = m
                    .splitters()
                    .into_iter()
                    .find(|(k, _, _)| *k == split)
                    .expect("the splitter");
                (line, vertical, (m.pane_split, m.split))
            };
            let mid = pos2(
                ((line.0 + line.2) / 2.0) as f32,
                ((line.1 + line.3) / 2.0) as f32,
            );
            let to = if vertical {
                pos2(mid.x - 80.0, mid.y)
            } else {
                pos2(mid.x, mid.y - 60.0)
            };
            drag(&mut s, mid, to);
            let m = s.model(view);
            let after = (m.pane_split, m.split);
            match split {
                // The left (upper) pane narrows.
                Split::Panes => assert!(
                    after.0 < before.0 && after.1 == before.1,
                    "{layout:?} {split:?}: {before:?} → {after:?}"
                ),
                // The difference pane grows.
                Split::Difference => assert!(
                    after.1 > before.1 && after.0 == before.0,
                    "{layout:?} {split:?}: {before:?} → {after:?}"
                ),
            }
            assert_eq!(m.options.geometry.offset, (0, 0), "{layout:?} {split:?}");
        }
        let kept = {
            let m = s.model(view);
            (m.pane_split, m.split)
        };
        let window = s.app.core.root();
        s.app
            .core
            .execute(window, crate::commands::CommandId::CloseTab);
        s.settle();
        let state = s.app.core.state();
        let value = |k: &str| match state.layout_value(crate::picture::VIEW, k) {
            Some(LayoutValue::Float(f)) => f,
            other => panic!("{k}: {other:?}"),
        };
        assert_eq!((value("pane_split"), value("difference_split")), kept);
        // The next view opens with them.
        let again = s.world.open(&mut s.app.core, &l, &r);
        s.settle();
        let m = s.model(again);
        assert_eq!((m.pane_split, m.split), kept);
        s.app
            .core
            .execute(window, crate::commands::CommandId::CloseTab);
        s.settle();
    }
}

/// The place a widget of the last frame kept (`crate::ui::picture::mark`).
fn place(s: &Screen, id: egui::Id) -> egui::Pos2 {
    s.ctx
        .memory(|m| m.data.get_temp::<egui::Rect>(id))
        .unwrap_or_else(|| panic!("no place for {id:?}"))
        .center()
}

/// A click at `p`.
fn click(s: &mut Screen, p: egui::Pos2) {
    s.input(vec![Event::PointerMoved(p)]);
    s.input(vec![pointer_button(p, PointerButton::Primary, true)]);
    s.input(vec![pointer_button(p, PointerButton::Primary, false)]);
    s.input(Vec::new());
}

fn key_with(key: egui::Key, modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Finding 17: the status bar's offset resets it (A13.13's *status click*), its zoom opens the Zoom list whose items
/// zoom.
#[test]
fn the_status_bar_offset_and_zoom_click() {
    use crate::status::StatusClick;
    let mut s = Screen::new();
    let (view, _) = at_actual_size(&mut s);
    let window = s.app.core.root();
    let items = s.app.core.status(window);
    assert_eq!(
        items.clicks,
        [
            Some(StatusClick::Run(crate::commands::CommandId::OffsetReset)),
            Some(StatusClick::ZoomList)
        ]
    );
    {
        let m = s.model(view);
        m.focus = hikasa_picture::Pane::Difference;
        let service = s.world.service.clone();
        let m = s.model(view);
        m.nudge(&service, 3, -2, false);
    }
    s.settle();
    assert_eq!(s.model(view).options.geometry.offset, (3, -2));
    let at = place(&s, crate::ui::status::item_id(0));
    click(&mut s, at);
    s.settle();
    assert_eq!(
        s.model(view).options.geometry.offset,
        (0, 0),
        "the offset's click resets it"
    );
    let at = place(&s, crate::ui::status::item_id(1));
    click(&mut s, at);
    // The list: Fit, Actual Size, 25 %, 50 %, 100 %, 200 % …
    let at = place(&s, crate::ui::status::item_id(1).with(5));
    click(&mut s, at);
    s.settle();
    assert_eq!(
        s.model(view).viewport.zoom,
        Zoom::Scale(2.0),
        "200 % from the list"
    );
}

/// Finding 17: a side's header has the path field (Enter reloads that side with the typed file), *Browse…* (the
/// browse dialog) and the format picker (*Auto* or a Picture format; a change decodes again).
#[test]
fn the_header_path_field_browse_and_format() {
    use crate::ui::picture::header_id;
    let mut s = Screen::new();
    let (view, _) = at_actual_size(&mut s);
    let other = png(&s.world, "other/c.png", 30, 20, base);
    // The path field: select all, type, Enter.
    let at = place(&s, header_id(view, 0, "path"));
    click(&mut s, at);
    s.input(vec![key_with(egui::Key::A, egui::Modifiers::COMMAND)]);
    s.input(vec![Event::Text(other.display().to_string())]);
    s.input(vec![key_with(egui::Key::Enter, egui::Modifiers::NONE)]);
    s.settle();
    {
        let m = s.model(view);
        assert_eq!(m.request().left, other.as_os_str(), "the typed file");
        let loaded = m.loaded().expect("loaded again");
        assert_eq!(loaded.pictures.left.info.width, 30);
    }
    // *Browse…*: the window's browse dialog for that side.
    let at = place(&s, header_id(view, 1, "browse"));
    click(&mut s, at);
    s.settle();
    let window = s.app.core.root();
    assert!(
        s.app
            .core
            .window_mut(window)
            .and_then(|w| w.dialogs.find_mut::<crate::browse::BrowseDialog>())
            .is_some(),
        "the browse dialog"
    );
    s.app
        .core
        .dialog_button(window, crate::dialog::ButtonRole::Cancel);
    s.settle();
    // The format picker: a Picture format, then Auto.
    let names = s.model(view).format_names().to_vec();
    let name = names.first().cloned().expect("an enabled Picture format");
    let at = place(&s, header_id(view, 1, "format"));
    click(&mut s, at);
    let at = place(&s, header_id(view, 1, &name));
    click(&mut s, at);
    s.settle();
    assert_eq!(
        s.model(view).side_format(hikasa_core::Side::Right),
        Some(name.as_str())
    );
    let at = place(&s, header_id(view, 1, "format"));
    click(&mut s, at);
    let at = place(&s, header_id(view, 1, "Auto"));
    click(&mut s, at);
    s.settle();
    assert_eq!(s.model(view).side_format(hikasa_core::Side::Right), None);
}

/// Finding 17 (A13.14's drops): a file dropped on a pane or its header replaces that side (the difference pane's:
/// the right); two files replace both; the view reloads.
#[test]
fn drops_replace_the_side_under_the_pointer() {
    use hikasa_core::Side;
    let mut s = Screen::new();
    let (view, _) = at_actual_size(&mut s);
    let a = png(&s.world, "drop/a.png", 12, 9, base);
    let b = png(&s.world, "drop/b.png", 14, 9, base);
    let (left, right, diff, header) = {
        let m = s.model(view);
        let at = |pane| {
            m.slots()
                .iter()
                .find(|x| x.pane == pane)
                .expect("pane")
                .picture
                .center()
        };
        let header = m.slots()[1].header;
        (
            at(hikasa_picture::Pane::Left),
            at(hikasa_picture::Pane::Right),
            at(hikasa_picture::Pane::Difference),
            (header.x + 4.0, header.y + 4.0),
        )
    };
    let m = s.model(view);
    assert_eq!(m.drop_side(Some(left)), Side::Left);
    assert_eq!(m.drop_side(Some(right)), Side::Right);
    assert_eq!(m.drop_side(Some(diff)), Side::Right);
    assert_eq!(m.drop_side(Some(header)), Side::Right);
    m.focus = hikasa_picture::Pane::Left;
    assert_eq!(m.drop_side(None), Side::Left);
    // Through the window: one file on the left pane, then two anywhere.
    let window = s.app.core.root();
    s.app.core.drop_paths(
        window,
        crate::dnd::DropTarget::PictureSide(Side::Left),
        vec![a.clone()],
    );
    s.settle();
    assert_eq!(s.model(view).request().left, a.as_os_str());
    s.app.core.drop_paths(
        window,
        crate::dnd::DropTarget::PictureSide(Side::Left),
        vec![b.clone(), a.clone()],
    );
    s.settle();
    let m = s.model(view);
    assert_eq!(
        (m.request().left.clone(), m.request().right.clone()),
        (b.into_os_string(), a.into_os_string())
    );
    assert_eq!(m.loaded().expect("loaded").pictures.left.info.width, 14);
}

/// Finding 17: the context menu the window draws shows the view's checks and radios.
#[test]
fn the_context_menu_shows_the_checks() {
    let mut s = Screen::new();
    let (view, at) = at_actual_size(&mut s);
    s.input(vec![Event::PointerMoved(at)]);
    s.input(vec![pointer_button(at, PointerButton::Secondary, true)]);
    s.input(vec![pointer_button(at, PointerButton::Secondary, false)]);
    s.input(Vec::new());
    let items = s
        .app
        .pictures
        .get(&view)
        .map(|st| st.menu_items.clone())
        .unwrap_or_default();
    let text = crate::menus::render_text(&[crate::menus::Menu {
        title: crate::menus::MenuTitle::Edit,
        items,
    }]);
    assert!(text.contains("[x] Transparency Checkerboard"), "{text}");
    assert!(text.contains("[x] Difference Pane"), "{text}");
    assert!(
        text.contains("[x] Fit to Window") || text.contains("[ ] Fit to Window"),
        "{text}"
    );
}
