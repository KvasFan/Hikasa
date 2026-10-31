//! The egui layer with a headless `egui::Context` (no window opens): what only the drawing layer decides — the
//! platform's close of the root viewport, egui's theme, the keyboard while a dialog is open, the log panel's
//! rows, the viewport builders.

use egui::{
    Event, Key, Modifiers, RawInput, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo,
};

use super::HikasaApp;
use crate::commands::CommandId;
use crate::startup::Deferred;
use crate::tests::{World, text_pair};
use crate::window::ViewModel;

pub(super) fn new_app(world: &World, ctx: &egui::Context) -> HikasaApp {
    let core = world.app(None).0;
    HikasaApp::with_context(
        ctx,
        core,
        world.mailbox.clone(),
        None,
        Deferred::new(|| {}),
        None,
    )
}

pub(super) fn raw(time: f64) -> RawInput {
    RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 800.0),
        )),
        time: Some(time),
        ..RawInput::default()
    }
}

/// One frame: the logic, then the drawing (what eframe does).
pub(super) fn frame(app: &mut HikasaApp, ctx: &egui::Context, raw: RawInput) -> egui::FullOutput {
    let mut out = ctx.run_ui(raw, |ui| {
        app.step(ui.ctx());
        app.draw(ui);
    });
    out.textures_delta.clear();
    out
}

pub(super) fn root_commands(out: &egui::FullOutput) -> Vec<ViewportCommand> {
    out.viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

pub(super) fn close_root(time: f64) -> RawInput {
    let mut r = raw(time);
    r.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            events: vec![ViewportEvent::Close],
            ..ViewportInfo::default()
        },
    );
    r
}

/// H1: the OS close of the root window while another window is open closes that window only: the root viewport's
/// close is cancelled (the next window moves into it) and the other window's views stay open.
#[test]
fn closing_the_root_window_keeps_the_other_windows() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let root = app.core.root();
    app.core.execute(root, CommandId::WindowNew);
    let second = app.core.windows()[1].id;
    let (l, r) = text_pair(&world);
    app.core.focus(second);
    let view = world.open(&mut app.core, &l, &r);
    assert_eq!(app.core.window_of(view), Some(second));
    frame(&mut app, &ctx, raw(0.0));

    let out = frame(&mut app, &ctx, close_root(0.1));
    assert!(
        root_commands(&out).contains(&ViewportCommand::CancelClose),
        "{:?}",
        root_commands(&out)
    );
    assert!(!app.closing && !app.core.quitting());
    assert_eq!(app.core.root(), second, "the second window is the root now");
    assert!(app.core.tab(view).is_some(), "its view stays open");
    assert!(world.shared.contains(view));
    frame(&mut app, &ctx, raw(0.2));
    assert!(app.core.tab(view).is_some());

    // The last window: the program quits, nothing is cancelled.
    let out = frame(&mut app, &ctx, close_root(0.3));
    assert!(!root_commands(&out).contains(&ViewportCommand::CancelClose));
    assert!(app.closing && app.core.quitting());
}

/// H1: a prompt (two tabs, `warn_when_closing_multiple_tabs`) keeps the window and cancels the close.
#[test]
fn a_close_prompt_cancels_the_platform_close() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let root = app.core.root();
    app.core.execute(root, CommandId::TabNew);
    frame(&mut app, &ctx, raw(0.0));
    let out = frame(&mut app, &ctx, close_root(0.1));
    assert!(root_commands(&out).contains(&ViewportCommand::CancelClose));
    assert_eq!(app.core.window(root).map(|w| w.dialogs.len()), Some(1));
}

/// M4: with a fixed light scheme and a dark OS, egui's theme is fixed to light and both of its styles carry the
/// palette, so nothing switches by itself.
#[test]
fn a_fixed_theme_holds_against_the_os_theme() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[appearance]\ntheme_mode = \"light\"\n",
    )]);
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    for (i, os) in [egui::Theme::Dark, egui::Theme::Light, egui::Theme::Dark]
        .into_iter()
        .enumerate()
    {
        let mut r = raw(i as f64);
        r.system_theme = Some(os);
        frame(&mut app, &ctx, r);
        assert_eq!(ctx.theme(), egui::Theme::Light, "OS {os:?}");
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let style = ctx.style_of(theme);
            assert!(!style.visuals.dark_mode, "{theme:?}");
            assert_eq!(style.visuals.panel_fill, app.palette.bg);
            assert_eq!(style.interaction.tooltip_delay, 0.0);
        }
    }
    // Following the OS: the core picks the scheme, egui's theme follows it.
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let mut r = raw(0.0);
    r.system_theme = Some(egui::Theme::Dark);
    frame(&mut app, &ctx, r);
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    assert!(ctx.style_of(egui::Theme::Light).visuals.dark_mode);
}

fn key(k: Key) -> Event {
    Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// A dialog takes the keyboard: the text view under it does not scroll on arrows and pages.
#[test]
fn a_text_view_under_a_dialog_does_not_scroll() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let lines: String = (0..2000).map(|i| format!("line {i}\n")).collect();
    let l = world.write("work/big-l.txt", &lines);
    let r = world.write("work/big-r.txt", &lines.replace("line 7", "LINE 7"));
    let view = world.open(&mut app.core, &l, &r);
    world.settle(&mut app.core);
    let mut t = 0.0;
    for _ in 0..5 {
        t += 0.05;
        frame(&mut app, &ctx, raw(t));
    }
    let top = |app: &HikasaApp| app.texts.get(&view).map(|s| s.top).expect("a text view");
    // Without a dialog the keys scroll (the panes have the focus).
    let before = top(&app);
    let mut r = raw(t + 0.05);
    r.events = vec![key(Key::PageDown)];
    frame(&mut app, &ctx, r);
    assert!(
        top(&app) > before,
        "PgDn scrolls: {} → {}",
        before,
        top(&app)
    );

    let window = app.core.root();
    app.core.execute(window, CommandId::About);
    assert!(app.core.ctx(window).dialog_open);
    let before = top(&app);
    t += 0.1;
    for k in [Key::ArrowDown, Key::PageDown, Key::End] {
        t += 0.05;
        let mut r = raw(t);
        r.events = vec![key(k)];
        frame(&mut app, &ctx, r);
    }
    assert_eq!(top(&app), before, "nothing scrolled under the dialog");
    // Esc closes the dialog (its key), not the view's.
    let mut r = raw(t + 0.05);
    r.events = vec![key(Key::Escape)];
    frame(&mut app, &ctx, r);
    assert!(!app.core.ctx(window).dialog_open);
}

/// L1: the log panel lays out only the rows on screen, whatever the book holds.
#[test]
fn the_log_panel_draws_only_visible_rows() {
    use hikasa_session::log::{LogBook, LogEntry, LogFilter, LogKind};
    let mut filter = LogFilter::from_options(&hikasa_session::settings::Options::default());
    filter.capacity = 50_000;
    let mut book = LogBook::new(filter, false);
    let world = World::new();
    let time = world.service.clock().now();
    for i in 0..20_000 {
        book.push(LogEntry {
            time: time.clone(),
            kind: LogKind::Summary,
            source: "test".into(),
            text: format!("entry {i}"),
        });
    }
    assert_eq!(book.len(), 20_000);
    let ctx = egui::Context::default();
    let pal = new_app(&world, &ctx).palette;
    let mut texts = 0;
    for t in [0.0, 0.1] {
        let mut out = ctx.run_ui(raw(t), |ui| {
            egui::Panel::bottom("log")
                .exact_size(300.0)
                .show(ui, |ui| super::log::show(ui, &book, &pal));
        });
        out.textures_delta.clear();
        texts = out
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Text(_)))
            .count();
    }
    assert!((5..100).contains(&texts), "{texts} text shapes");
}

/// L11: a window's viewport builder keeps the geometry it opened with; the platform's reports do not feed back.
#[test]
fn viewport_builders_keep_their_first_geometry() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let root = app.core.root();
    app.core.execute(root, CommandId::WindowNew);
    let second = app.core.windows()[1].id;
    let first = app.builder(second);
    let mut g = app.core.window(second).unwrap().geometry.clone();
    (g.x, g.y, g.width, g.height) = (400, 300, 900, 650);
    app.core.set_geometry(second, g, true);
    let again = app.builder(second);
    assert_eq!(again.inner_size, first.inner_size);
    assert_eq!(again.position, first.position);
    // The window becomes the root (swap) and leaves it again: a new builder from the model.
    app.apply(&ctx, vec![crate::app::Effect::CloseViewport(second)]);
    let fresh = app.builder(second);
    assert_eq!(fresh.inner_size, Some(egui::vec2(900.0, 650.0)));
    assert_eq!(fresh.position, Some(egui::pos2(400.0, 300.0)));
}

/// L7: a binding of a focus-bound key in the text panes fires there (an override of `Down` in the text view).
#[test]
fn text_panes_run_their_key_bindings() {
    let world = World::with_files(&[(
        "commands.toml",
        "schema_version = 1\n[view.text-compare.command.DiffSectionNext]\nshortcuts = [\"Down (in the text panes)\"]\n",
    )]);
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let lines: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let l = world.write("work/k-l.txt", &lines);
    let r = world.write(
        "work/k-r.txt",
        &lines
            .replace("line 100\n", "LINE 100\n")
            .replace("line 200\n", "LINE 200\n"),
    );
    let view = world.open(&mut app.core, &l, &r);
    world.settle(&mut app.core);
    let mut t = 0.0;
    for _ in 0..5 {
        t += 0.05;
        frame(&mut app, &ctx, raw(t));
    }
    let cursor = |app: &mut HikasaApp| match app.core.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Text(m)) => m.meta().cursor,
        _ => panic!("text"),
    };
    let before = cursor(&mut app);
    let mut r = raw(t + 0.05);
    r.events = vec![key(Key::ArrowDown)];
    frame(&mut app, &ctx, r);
    world.settle(&mut app.core);
    assert_ne!(cursor(&mut app), before, "Down ran DiffSectionNext");
}

/// W4c: every page of the Options dialog and every wizard draws, with its widgets on screen, and the capture takes
/// keys through the window's key path.
#[test]
fn the_settings_dialogs_draw() {
    use crate::dialog::ButtonRole;
    use crate::options::OptionsDialog;
    use crate::options::pages::PAGES;
    let world = World::new();
    world.write(
        "config/openwith.toml",
        "schema_version = 1\n[[app]]\ndescription = \"Ed\"\ncommand = \"ed %f\"\n",
    );
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let root = app.core.root();
    frame(&mut app, &ctx, raw(0.0));
    app.core.execute(root, CommandId::ProgramOptions);
    let mut t = 0.1;
    for page in PAGES {
        let d = app
            .core
            .window_mut(root)
            .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
            .expect("the dialog");
        d.show_page(page.id);
        d.select_command("About");
        d.open_with_new();
        frame(&mut app, &ctx, raw(t));
        t += 0.1;
        let out = frame(&mut app, &ctx, raw(t));
        t += 0.1;
        let texts = out
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Text(_)))
            .count();
        assert!(texts > 5, "{}: {texts} text shapes", page.id);
    }
    let d = app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
        .expect("the dialog");
    d.show_page("commands");
    d.begin_capture();
    let mut keyed = raw(t);
    keyed.events.push(Event::Key {
        key: Key::F9,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::CTRL,
    });
    frame(&mut app, &ctx, keyed);
    let d = app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
        .expect("the dialog");
    assert!(!d.capturing());
    assert!(
        d.commands.view["home"].command["About"]
            .shortcuts
            .as_ref()
            .is_some_and(|s| s == &["Ctrl+F9".to_owned()])
    );
    app.core.dialog_button(root, ButtonRole::Cancel);
    for command in [CommandId::Export, CommandId::Import, CommandId::Restore] {
        app.core.execute(root, command);
        for _ in 0..3 {
            t += 0.1;
            world.settle(&mut app.core);
            frame(&mut app, &ctx, raw(t));
        }
        app.core.dialog_button(root, ButtonRole::Cancel);
        assert!(app.core.window(root).is_some_and(|w| w.dialogs.is_empty()));
    }
}

/// M4: an open popup (a combo, a colour picker) takes Esc first; the dialog stays.
#[test]
fn an_open_popup_takes_esc_before_the_options_dialog() {
    use crate::options::OptionsDialog;
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let root = app.core.root();
    frame(&mut app, &ctx, raw(0.0));
    app.core.execute(root, CommandId::ProgramOptions);
    frame(&mut app, &ctx, raw(0.1));
    let open = |app: &mut crate::ui::HikasaApp| {
        app.core
            .window_mut(root)
            .is_some_and(|w| w.dialogs.find_mut::<OptionsDialog>().is_some())
    };
    egui::Popup::open_id(&ctx, egui::Id::unique("a popup of the page"));
    let esc = |t: f64| {
        let mut input = raw(t);
        input.events.push(Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        input
    };
    frame(&mut app, &ctx, esc(0.2));
    assert!(open(&mut app), "the popup took the key");
    egui::Popup::close_all(&ctx);
    frame(&mut app, &ctx, esc(0.3));
    assert!(!open(&mut app), "no popup, no edits: Esc closes");
}

/// L6: the button shows the stored colour unmultiplied: picking nothing changes nothing.
#[test]
fn a_colour_with_alpha_reads_back_as_stored() {
    use hikasa_session::settings::Colour;
    let value = Colour {
        rgb: [200, 100, 50],
        alpha: Some(128),
    };
    assert_eq!(crate::ui::options::colour_rgba(value), [200, 100, 50, 128]);
    assert_eq!(
        crate::ui::options::colour_rgba(Colour::hex(0x102030)),
        [0x10, 0x20, 0x30, 255]
    );
}

/// One frame of a bare toolbar; returns what it asked for and the rect it took.
fn toolbar_frame(
    ctx: &egui::Context,
    time: f64,
    events: Vec<Event>,
) -> (Vec<super::menus::ToolbarAction>, egui::Rect) {
    use super::menus::{Hover, ToolbarCtx, toolbar};
    let tables = crate::commands::CommandTables::default();
    let scope = crate::commands::Scope::Text;
    let items = crate::menus::toolbar_items(&tables, scope);
    let pal = super::theme::light();
    let enabled = |_: CommandId| true;
    let checked = |_: CommandId| None;
    let t = ToolbarCtx {
        sessions: &[],
        open_sessions: false,
        tables: &tables,
        scope,
        enabled: &enabled,
        checked: &checked,
        timing: crate::commands::HintTiming::default(),
        pal: &pal,
    };
    let mut result = (Vec::new(), egui::Rect::NOTHING);
    let mut input = raw(time);
    input.events = events;
    let mut output = ctx.run_ui(input, |ui| {
        let mut hover = Hover::default();
        let mut status = None;
        let actions = toolbar(ui, &t, &items, &mut hover, &mut status);
        result = (actions, ui.min_rect());
    });
    output.textures_delta.clear();
    result
}

pub(super) fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// Regression: the right-click area of UI-004 lay over the buttons and took their left clicks.
#[test]
fn a_left_click_reaches_the_toolbar_button_under_it() {
    let ctx = egui::Context::default();
    let (_, rect) = toolbar_frame(&ctx, 0.0, Vec::new());
    assert!(rect.width() > 20.0, "the toolbar drew nothing: {rect:?}");
    let pos = rect.left_center() + egui::vec2(12.0, 0.0);
    let _hover = toolbar_frame(&ctx, 0.1, vec![Event::PointerMoved(pos)]);
    let _hover = toolbar_frame(&ctx, 0.2, vec![Event::PointerMoved(pos)]);
    let _down = toolbar_frame(
        &ctx,
        0.3,
        vec![pointer_button(pos, egui::PointerButton::Primary, true)],
    );
    let (actions, _) = toolbar_frame(
        &ctx,
        0.4,
        vec![pointer_button(pos, egui::PointerButton::Primary, false)],
    );
    assert!(
        matches!(actions.as_slice(), [super::menus::ToolbarAction::Run(_)]),
        "the click ran no command"
    );
}

#[test]
fn a_right_click_on_the_toolbar_opens_its_menu() {
    let ctx = egui::Context::default();
    let (_, rect) = toolbar_frame(&ctx, 0.0, Vec::new());
    let pos = rect.left_center() + egui::vec2(12.0, 0.0);
    let _hover = toolbar_frame(&ctx, 0.1, vec![Event::PointerMoved(pos)]);
    let _hover = toolbar_frame(&ctx, 0.2, vec![Event::PointerMoved(pos)]);
    let _down = toolbar_frame(
        &ctx,
        0.3,
        vec![pointer_button(pos, egui::PointerButton::Secondary, true)],
    );
    let _up = toolbar_frame(
        &ctx,
        0.4,
        vec![pointer_button(pos, egui::PointerButton::Secondary, false)],
    );
    let (actions, _) = toolbar_frame(&ctx, 0.5, Vec::new());
    assert!(actions.is_empty(), "a right click must run nothing");
    assert!(egui::Popup::is_any_open(&ctx), "no menu opened");
}
