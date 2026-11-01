//! Simulated pointer events over the real drawing code: what a click reaches. A test that calls the model never
//! finds a widget something else was laid over, so these press and release the button at a widget's place on screen
//! and look at what happened; they also read back the layout that the screenshots showed wrong (W4 leftovers).

use egui::{Event, FullOutput, PointerButton, Pos2, Rect, Shape, pos2, vec2};
use hikasa_session::sessions::SessionType;

use super::HikasaApp;
use super::tests::{close_root, frame, new_app, pointer_button, raw, root_commands};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, Prompt};
use crate::home::{HomeModel, PanelTarget};
use crate::options::OptionsDialog;
use crate::tests::{World, text_pair};
use crate::window::{TabContent, ViewModel};

/// One text on screen.
#[derive(Debug, Clone)]
struct Drawn {
    text: String,
    rect: Rect,
}

fn collect(shape: &Shape, out: &mut Vec<Drawn>) {
    match shape {
        Shape::Text(t) => out.push(Drawn {
            text: t.galley.text().to_owned(),
            rect: t.galley.rect.translate(t.pos.to_vec2()),
        }),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

fn texts(out: &FullOutput) -> Vec<Drawn> {
    let mut drawn = Vec::new();
    for clipped in &out.shapes {
        collect(&clipped.shape, &mut drawn);
    }
    drawn
}

/// The window of a test, one frame at a time.
struct Screen {
    world: World,
    ctx: egui::Context,
    app: HikasaApp,
    time: f64,
}

impl Screen {
    fn with(world: World) -> Screen {
        let ctx = egui::Context::default();
        let app = new_app(&world, &ctx);
        let mut screen = Screen {
            world,
            ctx,
            app,
            time: 0.0,
        };
        screen.settle();
        screen
    }

    fn new() -> Screen {
        Screen::with(World::new())
    }

    fn input(&mut self, events: Vec<Event>) -> FullOutput {
        self.time += 0.1;
        let mut input = raw(self.time);
        input.events = events;
        frame(&mut self.app, &self.ctx, input)
    }

    /// Finishes the jobs and draws until the layout has settled (a new area sizes itself in its first frames).
    fn settle(&mut self) -> FullOutput {
        self.world.settle(&mut self.app.core);
        for _ in 0..3 {
            self.input(Vec::new());
        }
        self.input(Vec::new())
    }

    fn drawn(&mut self) -> Vec<Drawn> {
        let out = self.settle();
        texts(&out)
    }

    /// The place of the last `text` drawn (the last one is on top).
    fn find(&mut self, text: &str) -> Rect {
        let drawn = self.drawn();
        drawn
            .iter()
            .rev()
            .find(|d| d.text == text)
            .map(|d| d.rect)
            .unwrap_or_else(|| {
                let all: Vec<&str> = drawn.iter().map(|d| d.text.as_str()).collect();
                panic!("{text:?} is not on screen: {all:?}")
            })
    }

    fn press_release(&mut self, pos: Pos2, button: PointerButton) {
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![pointer_button(pos, button, true)]);
        self.input(vec![pointer_button(pos, button, false)]);
        self.input(vec![Event::PointerGone]);
    }

    fn click_at(&mut self, pos: Pos2) {
        self.press_release(pos, PointerButton::Primary);
        self.settle();
    }

    /// A left click on the text `text`.
    fn click(&mut self, text: &str) {
        let at = self.find(text).center();
        self.click_at(at);
    }

    fn drag(&mut self, from: Pos2, by: egui::Vec2) {
        let to = from + by;
        self.input(vec![Event::PointerMoved(from)]);
        self.input(vec![Event::PointerMoved(from)]);
        self.input(vec![pointer_button(from, PointerButton::Primary, true)]);
        for step in 1..=4 {
            let at = from + by * (step as f32 / 4.0);
            self.input(vec![Event::PointerMoved(at)]);
        }
        self.input(vec![pointer_button(to, PointerButton::Primary, false)]);
        self.input(vec![Event::PointerGone]);
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.input(vec![Event::Text(text.to_owned())]);
        self.settle();
    }

    fn root(&self) -> crate::window::WindowId {
        self.app.core.root()
    }

    fn execute(&mut self, id: CommandId) {
        let root = self.root();
        self.app.core.execute(root, id);
        self.settle();
    }

    fn tabs(&self) -> usize {
        self.app
            .core
            .window(self.root())
            .map_or(0, |w| w.tabs.len())
    }

    fn dialogs(&self) -> usize {
        self.app
            .core
            .window(self.root())
            .map_or(0, |w| w.dialogs.len())
    }

    fn options(&mut self) -> &mut OptionsDialog {
        let root = self.root();
        self.app
            .core
            .window_mut(root)
            .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
            .expect("the Options dialog")
    }

    fn home(&mut self) -> &mut HomeModel {
        let root = self.root();
        let view = self
            .app
            .core
            .window(root)
            .and_then(|w| w.tabs.first())
            .map(|t| t.id)
            .expect("a tab");
        match self.app.core.tab_mut(view).map(|t| &mut t.content) {
            Some(TabContent::Live(ViewModel::Home(home))) => home,
            _ => panic!("the first tab is not Home"),
        }
    }
}

// ---- the sweep: every interactive widget of the shell takes a left click ------------------------------------------

#[test]
fn the_tab_bar_takes_its_clicks() {
    let mut s = Screen::new();
    s.click("+");
    assert_eq!(s.tabs(), 2, "+ opens a tab");
    s.click("×");
    assert_eq!(s.tabs(), 1, "× closes a tab");
}

#[test]
fn a_click_activates_a_tab() {
    let mut s = Screen::new();
    s.execute(CommandId::TabNew);
    let active = |s: &Screen| s.app.core.window(s.root()).map(|w| w.active);
    assert_eq!(active(&s), Some(1));
    let tab = s
        .drawn()
        .into_iter()
        .find(|d| d.text == "Home")
        .expect("the first tab");
    s.click_at(tab.rect.center());
    assert_eq!(active(&s), Some(0));
}

#[test]
fn the_home_type_buttons_and_tree_rows_take_their_clicks() {
    let mut s = Screen::new();
    let buttons = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "Folder Compare")
        .collect::<Vec<_>>();
    // The type button above, the row of the tree under it.
    assert_eq!(buttons.len(), 2, "{buttons:?}");
    s.click_at(buttons[0].rect.center());
    assert_eq!(
        s.home().panel.target,
        PanelTarget::New(SessionType::FolderCompare)
    );
    let row = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "Text Compare")
        .max_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .expect("a row");
    s.click_at(row.rect.center());
    assert_eq!(
        s.home().panel.target,
        PanelTarget::New(SessionType::TextCompare)
    );
}

#[test]
fn the_home_compare_button_opens_the_paths() {
    let mut s = Screen::new();
    let (left, right) = text_pair(&s.world);
    s.home().panel.left = left.display().to_string();
    s.home().panel.right = right.display().to_string();
    s.click("Compare");
    s.world.settle(&mut s.app.core);
    let root = s.root();
    let view = s
        .app
        .core
        .window(root)
        .and_then(|w| w.tabs.first())
        .map(|t| t.id);
    let opened = s.app.core.tab_mut(view.expect("a tab")).map(|t| &t.content);
    assert!(
        matches!(opened, Some(TabContent::Live(ViewModel::Text(_)))),
        "the comparison replaced Home"
    );
}

#[test]
fn the_job_cancel_button_takes_its_click() {
    use crate::status::{JobStatus, StatusItems};
    use hikasa_session::ipc::ViewId;
    let ctx = egui::Context::default();
    let pal = super::theme::light();
    let items = StatusItems::default();
    let jobs = vec![JobStatus {
        view: ViewId(7),
        title: "compare".to_owned(),
        phase: None,
        fraction: None,
    }];
    let mut cancelled = Vec::new();
    let run = |time: f64, events: Vec<Event>, cancelled: &mut Vec<ViewId>| {
        let mut input = raw(time);
        input.events = events;
        let mut out = ctx.run_ui(input, |ui| {
            for action in super::status::bar(ui, &items, None, &jobs, &pal) {
                let super::status::StatusAction::Cancel(view) = action;
                cancelled.push(view);
            }
        });
        out.textures_delta.clear();
        texts(&out)
    };
    let place = |drawn: &[Drawn], text: &str| {
        drawn
            .iter()
            .find(|d| d.text == text)
            .map(|d| d.rect.center())
            .unwrap_or_else(|| panic!("{text} is not on screen: {drawn:?}"))
    };
    let mut time = 0.0;
    let mut step = |events: Vec<Event>, cancelled: &mut Vec<ViewId>| {
        time += 0.1;
        run(time, events, cancelled)
    };
    let drawn = step(Vec::new(), &mut cancelled);
    let drawn = if drawn.iter().any(|d| d.text == "1 job") {
        drawn
    } else {
        step(Vec::new(), &mut cancelled)
    };
    let at = place(&drawn, "1 job");
    step(vec![Event::PointerMoved(at)], &mut cancelled);
    step(
        vec![pointer_button(at, PointerButton::Primary, true)],
        &mut cancelled,
    );
    step(
        vec![pointer_button(at, PointerButton::Primary, false)],
        &mut cancelled,
    );
    step(Vec::new(), &mut cancelled);
    let drawn = step(Vec::new(), &mut cancelled);
    let at = place(&drawn, "Cancel");
    step(vec![Event::PointerMoved(at)], &mut cancelled);
    step(vec![Event::PointerMoved(at)], &mut cancelled);
    step(
        vec![pointer_button(at, PointerButton::Primary, true)],
        &mut cancelled,
    );
    step(
        vec![pointer_button(at, PointerButton::Primary, false)],
        &mut cancelled,
    );
    assert_eq!(cancelled, [ViewId(7)]);
}

/// A notice with its actions, as the watch raises it: a file changed under the unapplied edits of the Options dialog
/// (§1.4.7). The notice is the only way to answer, so it must work with the modal dialog open.
#[test]
fn the_notice_buttons_take_their_clicks_over_the_options_dialog() {
    use hikasa_session::settings::FormValue;
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    s.options()
        .set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    let options = s.world.write(
        "config/options.toml",
        "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n",
    );
    let change = hikasa_session::settings::ConfigChange {
        path: options,
        kind: hikasa_session::settings::ChangeKind::Changed,
    };
    let watch = vec![crate::watch::WatchEvent::Config(vec![change])];
    let effects = s.app.core.on_watch(watch);
    s.app.apply(&s.ctx.clone(), effects);
    let open = s
        .options()
        .field("tabs.open_new_tab_next_to_active")
        .cloned();
    assert_eq!(open.map(|f| f.value), Some(FormValue::Bool(false)));
    s.click("Reload (discard my edits)");
    let value = s
        .options()
        .field("tabs.open_new_tab_next_to_active")
        .cloned();
    assert_eq!(
        value.map(|f| f.value),
        Some(FormValue::Bool(true)),
        "the button reloaded the file"
    );
}

/// The same with a notice on screen before the dialog opened: its area must not stay under the modal.
#[test]
fn a_notice_shown_before_the_dialog_stays_clickable() {
    use hikasa_session::settings::FormValue;
    let mut s = Screen::new();
    let changed = |path| {
        crate::watch::WatchEvent::Config(vec![hikasa_session::settings::ConfigChange {
            path,
            kind: hikasa_session::settings::ChangeKind::Changed,
        }])
    };
    let theme = s.world.write("config/theme.toml", "schema_version = 1\n");
    let effects = s.app.core.on_watch(vec![changed(theme)]);
    s.app.apply(&s.ctx.clone(), effects);
    s.settle();
    s.execute(CommandId::ProgramOptions);
    s.options()
        .set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    let options = s.world.write(
        "config/options.toml",
        "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n",
    );
    let effects = s.app.core.on_watch(vec![changed(options)]);
    s.app.apply(&s.ctx.clone(), effects);
    s.click("Reload (discard my edits)");
    let value = s
        .options()
        .field("tabs.open_new_tab_next_to_active")
        .cloned();
    assert_eq!(value.map(|f| f.value), Some(FormValue::Bool(true)));
}

#[test]
fn a_notice_dismisses_with_its_button() {
    let mut s = Screen::new();
    let options = s.world.write(
        "config/options.toml",
        "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = false\n",
    );
    let change = hikasa_session::settings::ConfigChange {
        path: options,
        kind: hikasa_session::settings::ChangeKind::Changed,
    };
    let effects = s
        .app
        .core
        .on_watch(vec![crate::watch::WatchEvent::Config(vec![change])]);
    s.app.apply(&s.ctx.clone(), effects);
    let notice = |drawn: &[Drawn]| drawn.iter().any(|d| d.text.contains("Settings reloaded"));
    assert!(notice(&s.drawn()));
    let close = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "×")
        .max_by(|a, b| a.rect.left().total_cmp(&b.rect.left()))
        .expect("the notice's ×");
    s.click_at(close.rect.center());
    assert!(!notice(&s.drawn()), "dismissed");
}

#[test]
fn dialog_buttons_take_their_clicks() {
    let mut s = Screen::new();
    s.execute(CommandId::About);
    assert_eq!(s.dialogs(), 1);
    s.click("Close");
    assert_eq!(s.dialogs(), 0, "About closed");

    s.execute(CommandId::ProgramOptions);
    assert_eq!(s.dialogs(), 1);
    s.click("Cancel");
    assert_eq!(s.dialogs(), 0, "Options cancelled");
}

#[test]
fn a_prompt_and_its_check_box_take_their_clicks() {
    let mut s = Screen::new();
    let root = s.root();
    s.execute(CommandId::TabNew);
    let effects = s.app.core.close_window(root);
    s.app.apply(&s.ctx.clone(), effects);
    s.settle();
    assert_eq!(s.dialogs(), 1, "the multiple-close warning");
    s.click("Do not ask again");
    let top = s.app.core.window(root).and_then(|w| w.dialogs.top());
    let ticked = top
        .and_then(|d| d.as_any().downcast_ref::<Prompt>())
        .is_some_and(|p| p.ticked);
    assert!(ticked, "the check box took the click");
    s.click("Cancel");
    assert_eq!(s.dialogs(), 0, "the prompt closed");
    assert_eq!(s.tabs(), 2, "nothing closed");
}

#[test]
fn the_dialog_help_button_takes_its_click() {
    let mut s = Screen::new();
    s.execute(CommandId::About);
    s.click("?");
    let root = s.root();
    assert!(s.app.core.ctx(root).dialog_open, "the dialog stays");
    assert!(
        s.app.core.window(root).is_some_and(|w| w.help.is_some()),
        "F1's topic opened"
    );
}

/// The help of a dialog stays usable while the dialog is open: it is not under the modal.
#[test]
fn the_help_panel_takes_its_clicks_over_a_dialog() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    let root = s.root();
    let help = |s: &Screen| s.app.core.window(root).and_then(|w| w.help);
    s.click("?");
    assert_eq!(help(&s), Some("dialog-options"));
    s.click("Contents");
    assert_eq!(
        help(&s),
        Some(hikasa_session::help::INDEX_TOPIC),
        "the link works"
    );
    s.click("×");
    assert_eq!(help(&s), None, "× closes it");
    assert_eq!(s.dialogs(), 1, "the dialog stays");

    // Esc closes what opened last.
    s.click("?");
    s.input(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    s.settle();
    assert_eq!(help(&s), None);
    assert_eq!(s.dialogs(), 1, "Esc went to the help");
}

/// The message panel of a view.
#[test]
fn the_message_panel_closes_with_its_button() {
    let mut s = Screen::new();
    let (l, r) = text_pair(&s.world);
    let view = s.world.open(&mut s.app.core, &l, &r);
    s.settle();
    s.app
        .core
        .set_message(view, crate::status::Message::info("a message".to_owned()));
    let text = s.find("a message");
    let close = s
        .drawn()
        .into_iter()
        .rfind(|d| d.text == "×" && (d.rect.center().y - text.center().y).abs() < 12.0)
        .expect("its ×");
    s.click_at(close.rect.center());
    assert!(
        !s.drawn().iter().any(|d| d.text == "a message"),
        "the panel closed"
    );
}

/// A combo box inside a modal, also when its popup was on screen before the dialog opened again.
#[test]
fn a_combo_box_in_a_dialog_takes_its_clicks() {
    use hikasa_session::settings::{FormValue, enum_caption};
    let mut s = Screen::new();
    let path = "appearance.theme_mode";
    for pick in ["dark", "light"] {
        s.execute(CommandId::ProgramOptions);
        s.options().show_page("appearance");
        let current = s.options().field(path).map(|f| f.value.clone());
        let Some(FormValue::Text(current)) = current else {
            panic!("the field");
        };
        s.click(enum_caption(&current).unwrap_or(&current));
        s.click(enum_caption(pick).unwrap_or(pick));
        let now = s.options().field(path).map(|f| f.value.clone());
        assert_eq!(now, Some(FormValue::Text(pick.to_owned())));
        s.click("OK");
        assert_eq!(s.dialogs(), 0, "applied and closed");
    }
}

/// The steps of a wizard go by clicks.
#[test]
fn a_wizard_takes_its_clicks() {
    let mut s = Screen::new();
    import_wizard(&mut s, 1);
    s.click("Next");
    let top = s.drawn();
    assert!(
        top.iter().any(|d| d.text == "Choose what to import."),
        "{top:?}"
    );
    s.click("Include locked sessions");
    s.click("Cancel");
    assert_eq!(s.dialogs(), 0);
}

#[test]
fn the_options_page_tree_and_widgets_take_their_clicks() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    s.click("Tabs");
    assert_eq!(s.options().page, "tabs");
    let before = s
        .options()
        .field("tabs.hide_tab_bar_if_single_tab")
        .cloned()
        .expect("the field");
    let label = before.label;
    s.click(label);
    let after = s
        .options()
        .field("tabs.hide_tab_bar_if_single_tab")
        .cloned()
        .expect("the field");
    assert_ne!(before.value, after.value, "the check box took the click");
}

#[test]
fn the_menu_bar_opens_and_runs_an_item() {
    let mut s = Screen::new();
    s.click("Help");
    s.click("About");
    assert_eq!(s.dialogs(), 1, "About opened from the menu");
}

#[test]
fn the_view_panes_take_their_clicks() {
    let mut s = Screen::new();
    let lines: String = (0..400).map(|i| format!("line {i}\n")).collect();
    let l = s.world.write("work/t-l.txt", &lines);
    let r = s
        .world
        .write("work/t-r.txt", &lines.replace("line 300\n", "LINE 300\n"));
    let view = s.world.open(&mut s.app.core, &l, &r);
    s.settle();
    s.settle();
    let cursor = |s: &mut Screen| match s.app.core.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Text(m)) => m.meta().cursor,
        _ => panic!("text"),
    };
    let before = cursor(&mut s);
    let row = s.find("line 12");
    s.click_at(row.center());
    assert_ne!(cursor(&mut s), before, "a click moves the cursor");

    // The thumbnail is the strip at the left edge of the view (help/view-text-compare.md): a click scrolls.
    let top = s.app.texts.get(&view).map(|t| t.top).expect("a text view");
    let body = row.left_center();
    s.click_at(pos2(7.0, body.y + 400.0));
    let after = s.app.texts.get(&view).map(|t| t.top).expect("a text view");
    assert!(
        after > top + 10.0,
        "the thumbnail took the click: {top} → {after}"
    );
    // The scroll bar at the right edge.
    let top = after;
    s.click_at(pos2(1194.0, body.y + 10.0));
    let after = s.app.texts.get(&view).map(|t| t.top).expect("a text view");
    assert!(
        after < top,
        "the scroll bar took the click: {top} → {after}"
    );
    let help = include_str!("../../../../help/view-text-compare.md");
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        help.contains("thumbnail on the left"),
        "the help says where it is"
    );
}

#[test]
fn the_folder_tree_takes_its_clicks() {
    let mut s = Screen::new();
    s.world.write("work/a/x.txt", "1");
    s.world.write("work/b/x.txt", "longer");
    let (a, b) = (s.world.path("work/a"), s.world.path("work/b"));
    let view = s.world.open(&mut s.app.core, &a, &b);
    s.settle();
    s.settle();
    let selected = |s: &mut Screen| match s.app.core.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Folder(m)) => m.selected(),
        _ => panic!("folder"),
    };
    let before = selected(&mut s);
    s.click("x.txt");
    assert_ne!(selected(&mut s), before, "a click selects the row");
}

/// A large folder font widens the columns: size and time stay apart, on both sides.
#[test]
fn a_large_folder_font_keeps_the_columns_apart() {
    let mut s = Screen::with(World::with_files(&[(
        "theme.toml",
        "schema_version = 1\n[fonts.folder]\nfamily = \"\"\nsize = 17.0\nuse_system = false\n",
    )]));
    s.world.write("work/a/x.txt", "1");
    s.world.write("work/b/x.txt", "longer");
    let (a, b) = (s.world.path("work/a"), s.world.path("work/b"));
    s.world.open(&mut s.app.core, &a, &b);
    s.settle();
    s.settle();
    let drawn = s.drawn();
    let at = |text: &str| -> Vec<Rect> {
        drawn
            .iter()
            .filter(|d| d.text == text)
            .map(|d| d.rect)
            .collect()
    };
    let times: Vec<Rect> = drawn
        .iter()
        .filter(|d| d.text.len() == 19 && d.text.as_bytes()[4] == b'-')
        .map(|d| d.rect)
        .collect();
    let sizes = [at("1"), at("6")].concat();
    assert_eq!((sizes.len(), times.len()), (2, 2), "{drawn:?}");
    for size in sizes {
        let time = times
            .iter()
            .find(|t| (t.top() - size.top()).abs() < 1.0 && t.left() > size.center().x - 200.0)
            .expect("a time on the row");
        assert!(
            size.right() < time.left(),
            "the size ends at {} and the time starts at {}",
            size.right(),
            time.left()
        );
    }
}

// ---- W4 leftovers: the OS close button, the search, the page's reasons -----------------------------------------------

/// D-112: with unapplied edits the OS close button asks like Esc; the answer decides.
#[test]
fn the_os_close_button_asks_about_unapplied_edits() {
    use hikasa_session::settings::FormValue;
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    let time = s.time + 0.1;
    let out = frame(&mut s.app, &s.ctx, close_root(time));
    assert!(root_commands(&out).contains(&egui::ViewportCommand::CancelClose));
    assert_eq!(s.dialogs(), 0, "nothing to lose: a plain Cancel");

    s.execute(CommandId::ProgramOptions);
    s.options()
        .set_field("tabs.open_new_tab_next_to_active", FormValue::Bool(false));
    s.time += 0.1;
    let out = frame(&mut s.app, &s.ctx, close_root(s.time));
    assert!(root_commands(&out).contains(&egui::ViewportCommand::CancelClose));
    assert_eq!(s.dialogs(), 2, "the question over the dialog");
    assert!(!s.app.core.quitting());
    s.click("Keep editing");
    assert_eq!(s.dialogs(), 1);
    assert!(s.options().dirty(), "the edit is still there");

    s.time += 0.1;
    frame(&mut s.app, &s.ctx, close_root(s.time));
    assert_eq!(s.dialogs(), 2);
    s.click("Discard");
    assert_eq!(s.dialogs(), 0, "closed without the edit");
    assert_eq!(s.tabs(), 1, "the window stays");
}

/// The right pane follows the search: the page shown is one the tree lists, and without a match no page is shown.
#[test]
fn the_options_search_moves_the_page() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    let first = s.options().page;
    assert_eq!(first, "startup");
    let title = "Startup";
    let titles = |d: &[Drawn], text: &str| d.iter().filter(|d| d.text == text).count();
    assert_eq!(
        titles(&s.drawn(), title),
        2,
        "the tree row and the page title"
    );
    s.click("Search");
    s.type_text("log panel");
    assert_eq!(s.options().page, "tweaks.log_panel");
    let drawn = s.drawn();
    assert_eq!(titles(&drawn, "Log Panel"), 2, "{drawn:?}");
    assert_eq!(titles(&drawn, title), 0, "the old page is gone: {drawn:?}");
    s.type_text("zzz");
    let drawn = s.drawn();
    assert_eq!(titles(&drawn, "No match."), 1, "{drawn:?}");
    assert_eq!(
        titles(&drawn, "No page matches the search."),
        1,
        "{drawn:?}"
    );
    assert_eq!(titles(&drawn, "Log Panel"), 0, "no page: {drawn:?}");
    // A page that matches stays, whichever page matches first.
    s.options().search.clear();
    s.options().show_page("tabs");
    s.click("Search");
    s.type_text("t");
    assert_eq!(s.options().page, "tabs");
}

#[test]
fn the_page_says_why_a_file_cannot_be_overwritten() {
    let world = World::with_files(&[("commands.toml", "schema_version = 99\n")]);
    let mut s = Screen::with(world);
    s.execute(CommandId::ProgramOptions);
    let drawn = s.drawn();
    assert!(
        drawn
            .iter()
            .any(|d| d.text.starts_with("commands.toml cannot be read") && d.text.contains("99")),
        "{drawn:?}"
    );
}

// ---- W4 leftovers: what the screenshots showed --------------------------------------------------------------------

/// Every Menu box of the commands list stands in one column, with or without a Toolbar box beside it.
#[test]
fn the_menu_column_of_the_commands_list_is_straight() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    s.options().show_page("commands");
    s.options().select_view("folder-compare");
    let drawn = s.drawn();
    let column = |text: &str| {
        let mut lefts: Vec<i32> = drawn
            .iter()
            .filter(|d| d.text == text)
            .map(|d| d.rect.left().round() as i32)
            .collect();
        lefts.sort_unstable();
        lefts.dedup();
        lefts
    };
    assert_eq!(column("Menu").len(), 1, "{:?}", column("Menu"));
    assert_eq!(column("Toolbar").len(), 1, "{:?}", column("Toolbar"));
    assert!(
        drawn.iter().filter(|d| d.text == "Menu").count()
            > drawn.iter().filter(|d| d.text == "Toolbar").count(),
        "the list holds rows without a Toolbar box"
    );
}

#[test]
fn the_log_panel_counts_in_the_singular_too() {
    use hikasa_session::log::{LogBook, LogEntry, LogFilter, LogKind};
    let world = World::new();
    let filter = LogFilter::from_options(&hikasa_session::settings::Options::default());
    let mut book = LogBook::new(filter, false);
    let pal = super::theme::light();
    let ctx = egui::Context::default();
    let mut counts = Vec::new();
    for time in 0..3 {
        if time > 0 {
            book.push(LogEntry {
                time: world.service.clock().now(),
                kind: LogKind::Summary,
                source: "test".into(),
                text: "entry".into(),
            });
        }
        let mut out = ctx.run_ui(raw(f64::from(time)), |ui| {
            super::log::show(ui, &book, &pal);
        });
        out.textures_delta.clear();
        counts.extend(
            texts(&out)
                .into_iter()
                .map(|d| d.text)
                .filter(|t| t.ends_with(" entries") || t.ends_with("1 entry")),
        );
    }
    assert_eq!(counts, ["0 entries", "1 entry", "2 entries"]);
}

/// The place of a dialog of `depth` (the frame of its area).
fn dialog_rect(s: &Screen, depth: usize) -> Rect {
    egui::AreaState::load(&s.ctx, egui::Id::unique(("hk-dialog", depth)))
        .expect("the dialog's area")
        .rect()
}

/// The Import wizard on a package, on `step`.
fn import_wizard(s: &mut Screen, sessions: usize) -> crate::window::WindowId {
    use crate::window::{CommandArg, Invocation};
    use hikasa_session::package::ExportRequest;
    use hikasa_session::sessions::{Session, SessionRef};
    use hikasa_session::settings::Overwrite;
    let store = s.world.service.session_store();
    let kind = SessionType::TextCompare;
    let session = Session::new(kind, &store.defaults(kind).expect("defaults"));
    for i in 0..sessions {
        let reference = SessionRef::parse(&format!("session {i}")).expect("a reference");
        store
            .save(&reference, &session, Overwrite::IfUnchanged)
            .expect("save");
    }
    let package = s.world.path("work/p.hikasa-pkg");
    s.world
        .service
        .settings_export(&ExportRequest::default(), &package)
        .expect("export");
    let root = s.root();
    let open = Invocation::with(CommandId::Import, CommandArg::Paths(vec![package]));
    let effects = s.app.core.execute(root, open);
    s.app.apply(&s.ctx.clone(), effects);
    s.settle();
    root
}

/// The buttons stand at the bottom of the wizard on every step, and the list takes the space above what is under it.
#[test]
fn the_wizard_buttons_sit_at_the_bottom_and_the_list_fills_the_space() {
    let mut s = Screen::new();
    let root = import_wizard(&mut s, 30);
    let bottom_gap = |s: &mut Screen, text: &str| {
        let button = s.find(text);
        dialog_rect(s, 1).bottom() - button.bottom()
    };
    assert!(bottom_gap(&mut s, "Next") < 24.0, "step 1");
    let next = s.find("Next");
    let size = dialog_rect(&s, 1).size();
    let effects = s.app.core.dialog_button(root, ButtonRole::Next);
    s.app.apply(&s.ctx.clone(), effects);
    s.settle();
    let turned = s.find("Next");
    assert!(
        (turned.top() - next.top()).abs() < 1.0,
        "same place on step 2"
    );
    assert_eq!(dialog_rect(&s, 1).size(), size, "same size on step 2");

    let top = s.find("Choose what to import.").bottom();
    let under = s.find("Include locked sessions");
    assert!(
        under.top() - top > 300.0,
        "the list is {} high",
        under.top() - top
    );
    assert!(
        turned.top() - under.bottom() < 70.0,
        "no blank space under it"
    );
}

#[test]
fn a_short_window_keeps_the_wizard_buttons_on_screen() {
    let mut s = Screen::new();
    import_wizard(&mut s, 3);
    s.settle();
    s.time += 0.1;
    let short = |time| {
        let mut input = raw(time);
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 420.0)));
        input
    };
    for _ in 0..4 {
        s.time += 0.1;
        frame(&mut s.app, &s.ctx, short(s.time));
    }
    s.time += 0.1;
    let out = frame(&mut s.app, &s.ctx, short(s.time));
    let drawn = texts(&out);
    let next = drawn.iter().find(|d| d.text == "Next").expect("the button");
    assert!(next.rect.bottom() < 420.0, "{next:?}");
}

/// The title bar moves a dialog; what is in it keeps working, and the dialog stays on screen.
#[test]
fn the_options_dialog_moves_by_its_title() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    let title = s.find("Options").center();
    let cancel = s.find("Cancel");
    s.drag(title, vec2(-120.0, 70.0));
    let moved = s.find("Cancel");
    let by = moved.center() - cancel.center();
    assert!((by - vec2(-120.0, 70.0)).length() < 1.0, "moved by {by:?}");

    // The help button in the bar is still a button.
    s.click("?");
    assert!(
        s.app
            .core
            .window(s.root())
            .is_some_and(|w| w.help.is_some()),
        "F1's topic opened"
    );
    s.click("×");
    // Far past the edge: the dialog stops at it.
    let title = s.find("Options").center();
    s.drag(title, vec2(-3000.0, -3000.0));
    let rect = dialog_rect(&s, 1);
    assert_eq!(rect.left_top().round(), Pos2::ZERO, "{rect:?}");
    // A button at the new place takes its click.
    s.click("Cancel");
    assert_eq!(s.dialogs(), 0);
}

/// A drag on the page does not move the dialog: only the title bar does.
#[test]
fn a_drag_on_the_page_leaves_the_dialog_where_it_is() {
    let mut s = Screen::new();
    s.execute(CommandId::ProgramOptions);
    let before = dialog_rect(&s, 1);
    let page = pos2(before.right() - 40.0, before.bottom() - 100.0);
    s.drag(page, vec2(-100.0, -100.0));
    assert_eq!(dialog_rect(&s, 1), before);
}

/// The marks of a menu sit on the line of their label.
#[test]
fn menu_marks_sit_on_their_line() {
    use crate::menus::{Check, MenuItem};
    use crate::window::Invocation;
    let entry = |caption: &str, check| MenuItem::Command {
        invocation: Invocation::new(CommandId::About),
        caption: caption.to_owned(),
        shortcut: None,
        check,
        enabled: true,
        description: String::new(),
    };
    let list = vec![
        entry("Toggle item", Check::Toggle(true)),
        entry("Radio item", Check::Radio(true)),
        entry("Off item", Check::Toggle(false)),
    ];
    let ctx = egui::Context::default();
    let mut drawn = Vec::new();
    for time in 0..3 {
        let mut out = ctx.run_ui(raw(f64::from(time)), |ui| {
            let (mut run, mut hint) = (Vec::new(), None);
            super::menus::items(ui, &list, &mut run, &mut hint);
        });
        out.textures_delta.clear();
        drawn = texts(&out);
    }
    for (mark, label) in [("✔", "Toggle item"), ("●", "Radio item")] {
        let place = |text: &str| {
            drawn
                .iter()
                .find(|d| d.text == text)
                .map(|d| d.rect.center().y)
                .unwrap_or_else(|| panic!("{text}: {drawn:?}"))
        };
        assert!(
            (place(mark) - place(label)).abs() < 1.0,
            "{mark} at {}, {label} at {}",
            place(mark),
            place(label)
        );
    }
    let left = |text: &str| drawn.iter().find(|d| d.text == text).map(|d| d.rect.left());
    assert_eq!(
        left("Toggle item"),
        left("Off item"),
        "the labels stay in a column"
    );
}

/// A5.19: a double-click on a closed archive row opens it as a folder; the zipper icon is on the row.
#[test]
fn a_double_click_opens_an_archive_row() {
    let world = World::new();
    let write = |rel: &str, bytes: &[u8]| {
        let path = world.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let zip = crate::tests_shell::stored_zip(&[("member.txt", b"inside\n")]);
    write("l/a.zip", &zip);
    write(
        "r/a.zip",
        &crate::tests_shell::stored_zip(&[("member.txt", b"other\n")]),
    );
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    let row = s.find("a.zip");
    let pos = row.center();
    // Two presses within the double-click time.
    for _ in 0..2 {
        s.input(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        s.input(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        s.time += 0.05;
    }
    s.settle();
    s.world.settle(&mut s.app.core);
    s.settle();
    let model = crate::tests::folder_model(&mut s.app.core, view);
    let rules = model.prepared().expect("prepared").rules().clone();
    assert_eq!(rules.open_archives, ["a.zip"]);
    s.settle();
    s.find("member.txt");
}
