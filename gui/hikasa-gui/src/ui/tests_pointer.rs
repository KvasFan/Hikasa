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
    // Rows with and without a Toolbar box in the visible part of the list (W9a's entries pushed the toolbar rows
    // further down): "Next Difference" has an icon, "Next Item on This Side" none.
    s.options().set_command_search("Next");
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
    assert_eq!(model.open_archives(), Some(&["a.zip".to_owned()][..]));
    s.settle();
    s.find("member.txt");
}

/// A6.34 (M2 W6d): a row is deleted by its dialog, with the pointer and the keyboard only: a click selects it, Del opens
/// the Delete dialog, a click on its check box turns the recycle bin off (checked before OK: no test calls the real
/// bin), OK, then Yes in the confirmation; the file and its row are gone.
#[test]
fn a6_34_a_row_is_deleted_by_its_dialog() {
    use crate::op_dialogs::{DeleteDialog, PlanConfirmDialog};
    let world = World::new();
    world.write("l/keep.txt", "k\n");
    world.write("r/keep.txt", "k\n");
    let gone = world.write("r/gone.txt", "bye\n");
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    s.click("gone.txt");
    let model = crate::tests::folder_model(&mut s.app.core, view);
    assert!(model.selected().is_some(), "the click selects the row");
    s.input(vec![Event::Key {
        key: egui::Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    s.settle();
    assert_eq!(s.dialogs(), 1, "Del opens the Delete dialog");
    s.click("Use recycle bin if possible");
    let root = s.root();
    let bin = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<DeleteDialog>())
        .map(|d| d.use_bin());
    assert_eq!(bin, Some(false), "the check box turned the bin off");
    s.click("OK");
    s.world.settle(&mut s.app.core);
    s.settle();
    let counts = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<PlanConfirmDialog>())
        .map(|d| d.counts().to_vec());
    assert_eq!(counts, Some(vec!["remove: 1 items".to_owned()]));
    s.click("Yes");
    s.world.settle(&mut s.app.core);
    s.settle();
    assert_eq!(s.dialogs(), 0);
    assert!(!gone.exists(), "the file is deleted");
    let drawn = s.drawn();
    assert!(
        !drawn.iter().any(|d| d.text == "gone.txt"),
        "its row is gone"
    );
}

/// A9.2, A9.4 (M2 W9a): a header click sorts by that column on that side, the arrow on that side only; a click on a
/// right cell selects the row on the right alone; a swipe from a left cell across the centre selects both sides.
#[test]
fn a9_header_sort_side_clicks_and_a_swipe_across_the_centre() {
    use hikasa_folder::select::SideSet;
    let world = World::new();
    for (name, l, r) in [
        ("a.txt", "1", "1"),
        ("b.txt", "22", "2"),
        ("c.txt", "333", "3"),
    ] {
        world.write(&format!("l/{name}"), l);
        world.write(&format!("r/{name}"), r);
    }
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    s.execute(CommandId::FavCompareAll);
    s.settle();
    // The Size headers: two (one a side); a click on the right one sorts the right side.
    let headers: Vec<Rect> = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == crate::folder::columns::title(crate::folder::columns::ColumnId::Size))
        .map(|d| d.rect)
        .collect();
    assert_eq!(headers.len(), 2, "one Size header a side");
    let right_header = headers
        .iter()
        .copied()
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    s.click_at(right_header.center());
    s.settle();
    let model = crate::tests::folder_model(&mut s.app.core, view);
    let sort = model.columns().sort.expect("sorted");
    assert_eq!(
        (sort.column, sort.side, sort.descending),
        (
            crate::folder::columns::ColumnId::Size,
            hikasa_core::Side::Right,
            false
        )
    );
    let arrows: Vec<String> = s
        .drawn()
        .into_iter()
        .filter(|d| {
            d.text.starts_with(crate::folder::columns::title(
                crate::folder::columns::ColumnId::Size,
            ))
        })
        .map(|d| d.text)
        .collect();
    assert_eq!(
        arrows.iter().filter(|t| t.ends_with('▲')).count(),
        1,
        "{arrows:?}"
    );
    // A click on the right cell of b.txt: the right side alone.
    let names: Vec<Rect> = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "b.txt")
        .map(|d| d.rect)
        .collect();
    assert_eq!(names.len(), 2);
    let right_b = names
        .iter()
        .copied()
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    let left_b = names
        .iter()
        .copied()
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    s.click_at(right_b.center());
    s.settle();
    let model = crate::tests::folder_model(&mut s.app.core, view);
    let picked: Vec<SideSet> = model
        .selection()
        .items
        .iter()
        .map(|(_, sides)| sides)
        .collect();
    assert_eq!(picked, [SideSet::RIGHT]);
    // A swipe from a.txt's left cell down to c.txt across the centre: three rows, both sides.
    let a_left = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "a.txt")
        .map(|d| d.rect)
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    let c_right = s
        .drawn()
        .into_iter()
        .filter(|d| d.text == "c.txt")
        .map(|d| d.rect)
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    let _ = left_b;
    s.drag(a_left.center(), c_right.center() - a_left.center());
    s.settle();
    let model = crate::tests::folder_model(&mut s.app.core, view);
    let picked: Vec<SideSet> = model
        .selection()
        .items
        .iter()
        .map(|(_, sides)| sides)
        .collect();
    assert_eq!(
        picked,
        [SideSet::all(); 3],
        "a swipe across the centre selects both sides"
    );
}

/// A9.1 (D-234): a header dragged moves its column, a separator dragged resizes the column, a double-click on it
/// fits the column to the widest rendered cell.
#[test]
fn a9_1_header_drags_move_and_resize_a_double_click_fits() {
    use crate::folder::columns::{ColumnId, title};
    let world = World::new();
    for (name, l, r) in [("a.txt", "1", "1"), ("b.txt", "22", "2")] {
        world.write(&format!("l/{name}"), l);
        world.write(&format!("r/{name}"), r);
    }
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    s.execute(CommandId::FavCompareAll);
    s.settle();
    let leftmost = |s: &mut Screen, text: &str| {
        s.drawn()
            .into_iter()
            .filter(|d| d.text == text)
            .map(|d| d.rect)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("drawn")
    };
    // Modified dropped left of Size: Name, Modified, Size.
    let size = leftmost(&mut s, title(ColumnId::Size));
    let modified = leftmost(&mut s, title(ColumnId::Modified));
    let to = egui::pos2(size.left() + 2.0, size.center().y);
    s.drag(modified.center(), to - modified.center());
    let m = crate::tests::folder_model(&mut s.app.core, view);
    assert_eq!(
        m.shown_columns(),
        [ColumnId::Name, ColumnId::Modified, ColumnId::Size]
    );
    assert_eq!(m.columns().sort, None, "a drag is not a click");
    // Size's left separator (8 points left of its title) dragged 30 points left: 30 points wider.
    let before = m.columns().width(ColumnId::Size);
    let size = leftmost(&mut s, title(ColumnId::Size));
    let grip = egui::pos2(size.left() - 8.0, size.center().y);
    s.drag(grip, egui::vec2(-30.0, 0.0));
    let m = crate::tests::folder_model(&mut s.app.core, view);
    let wider = m.columns().width(ColumnId::Size);
    assert!((wider - (before + 30.0)).abs() < 1.5, "{before} → {wider}");
    // A double-click on it: the widest drawn size text and the margin, clamped to the minimum.
    let size = leftmost(&mut s, title(ColumnId::Size));
    let grip = egui::pos2(size.left() - 8.0, size.center().y);
    s.input(vec![Event::PointerMoved(grip)]);
    s.input(vec![
        pointer_button(grip, PointerButton::Primary, true),
        pointer_button(grip, PointerButton::Primary, false),
        pointer_button(grip, PointerButton::Primary, true),
        pointer_button(grip, PointerButton::Primary, false),
    ]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    let m = crate::tests::folder_model(&mut s.app.core, view);
    assert_eq!(
        m.columns().width(ColumnId::Size),
        crate::folder::columns::min_width(ColumnId::Size),
        "two-digit sizes fit in the minimum"
    );
}

/// §1.9.3 (review finding 8): with every column shown a side is wider than its half: Name keeps its 80 points, the
/// sides scroll horizontally together (clamped to the content), and the header moves with them.
#[test]
fn a9_1_a_side_wider_than_its_half_scrolls_both_sides_together() {
    use crate::folder::columns::{ColumnId, min_width, title};
    let world = World::new();
    world.write("l/a.txt", "1");
    world.write("r/a.txt", "2");
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    let spans = |s: &Screen| s.app.folders[&view].spans.clone();
    // Few columns: nothing scrolls, the fixed columns end at the side's right edge.
    let narrow = spans(&s);
    assert!(narrow[0][0].1.span() >= min_width(ColumnId::Name));
    s.app
        .folders
        .get_mut(&view)
        .expect("the view's state")
        .hscroll = 500.0;
    s.settle();
    assert_eq!(s.app.folders[&view].hscroll, 0.0, "nothing to scroll");
    // Every column: Name at its minimum, the content wider than the side.
    crate::tests::folder_model(&mut s.app.core, view).change_columns(|c| {
        for id in ColumnId::ALL {
            if !c.is_visible(id) && id != ColumnId::Path {
                c.toggle(id);
            }
        }
    });
    s.settle();
    let wide = spans(&s);
    let side_w = narrow[0].last().expect("a column").1.max - narrow[0][0].1.min;
    let content = wide[0].last().expect("a column").1.max - wide[0][0].1.min;
    assert!(content > side_w, "{content} > {side_w}");
    assert!(
        (wide[0][0].1.span() - min_width(ColumnId::Name)).abs() < 0.5,
        "Name keeps its minimum"
    );
    let name_headers = |s: &mut Screen| -> Vec<f32> {
        let mut xs: Vec<f32> = s
            .drawn()
            .into_iter()
            .filter(|d| d.text == title(ColumnId::Name))
            .map(|d| d.rect.left())
            .collect();
        xs.sort_by(f32::total_cmp);
        xs
    };
    let before = name_headers(&mut s);
    assert_eq!(before.len(), 2);
    // Scrolled by 40 points: both sides' columns and headers move 40 points left.
    s.app
        .folders
        .get_mut(&view)
        .expect("the view's state")
        .hscroll = 40.0;
    s.settle();
    let scrolled = spans(&s);
    for k in 0..2 {
        assert!(
            (scrolled[k][0].1.min - (wide[k][0].1.min - 40.0)).abs() < 0.01,
            "side {k}"
        );
    }
    let after = name_headers(&mut s);
    // A header partly scrolled out is clipped, not moved: compare the right side's, which starts inside.
    assert!(
        (after[1] - (before[1] - 40.0)).abs() < 0.5,
        "{before:?} → {after:?}"
    );
    // Past the end: clamped so the last column ends at the side's right edge.
    s.app
        .folders
        .get_mut(&view)
        .expect("the view's state")
        .hscroll = 1.0e6;
    s.settle();
    let end = spans(&s);
    let hscroll = s.app.folders[&view].hscroll;
    assert!((hscroll - (content - side_w)).abs() < 0.5, "{hscroll}");
    assert!(
        (end[0].last().expect("a column").1.max - narrow[0].last().expect("a column").1.max).abs()
            < 0.5
    );
}

// ---- M2 W9c (A9.25 through the drawing code, headless): context menus, drags, the clipboard keys, OS drops ---------

/// A folder view of `l` (a.txt, only-left.txt, d/) and `r` (a.txt, d/), every row shown, with a fixed clipboard.
fn w9c_screen() -> (
    Screen,
    hikasa_session::ipc::ViewId,
    std::sync::Arc<hikasa_session::clipboard::FixedClipboard>,
) {
    let world = World::new();
    world.write("l/a.txt", "left\n");
    world.write("r/a.txt", "right\n");
    world.write("l/only-left.txt", "l\n");
    world.write("l/d/x.txt", "x\n");
    world.write("r/d/x.txt", "x\n");
    let (left, right) = (world.path("l"), world.path("r"));
    let mut s = Screen::with(world);
    let fixed = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text("t"));
    s.app
        .core
        .set_script_clipboard(std::sync::Arc::clone(&fixed) as _);
    let view = s.world.open(&mut s.app.core, &left, &right);
    s.world.settle(&mut s.app.core);
    s.settle();
    s.execute(CommandId::FavCompareAll);
    (s, view, fixed)
}

/// The place of the `n`-th (from 0) drawing of `text`.
fn nth(s: &mut Screen, text: &str, n: usize) -> Rect {
    s.drawn()
        .iter()
        .filter(|d| d.text == text)
        .nth(n)
        .map(|d| d.rect)
        .unwrap_or_else(|| panic!("{text:?} #{n} is not on screen"))
}

#[test]
fn w9c_a_right_click_opens_the_pane_menu_and_its_entry_runs() {
    use hikasa_folder::select::SideSet;
    let (mut s, view, fixed) = w9c_screen();
    s.click("only-left.txt");
    // A right-click on the right side of a.txt: that row on that side is the selection, the right pane's menu opens.
    let right = nth(&mut s, "a.txt", 1).center();
    s.press_release(right, PointerButton::Secondary);
    s.settle();
    let model = crate::tests::folder_model(&mut s.app.core, view);
    let node = model.selected().expect("selected");
    assert_eq!(model.selection().items.get(node), SideSet::RIGHT);
    let drawn: Vec<String> = s.drawn().into_iter().map(|d| d.text).collect();
    for entry in [
        "Copy to Other Side",
        "Copy Filename",
        "File Manager",
        "Paste",
    ] {
        assert!(drawn.iter().any(|t| t == entry), "{entry}: {drawn:?}");
    }
    // Its Copy Filename copies the right item's spec.
    s.click("Copy Filename");
    s.world.settle(&mut s.app.core);
    let payloads = fixed.payloads();
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads[0].text.ends_with("a.txt"),
        "{:?}",
        payloads[0].text
    );
    assert!(payloads[0].text.contains("/r/") || payloads[0].text.contains("\\r\\"));
}

#[test]
fn w9c_a_selected_row_dragged_to_the_other_pane_asks_to_copy_it() {
    let (mut s, _view, _) = w9c_screen();
    s.click("only-left.txt");
    let from = nth(&mut s, "only-left.txt", 0).center();
    // Across the centre into the right pane, below the rows.
    let to_x = 900.0;
    s.drag(from, vec2(to_x - from.x, 0.0));
    s.world.settle(&mut s.app.core);
    s.settle();
    let root = s.root();
    let asked = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<crate::op_dialogs::PlanConfirmDialog>())
        .is_some();
    assert!(asked, "Copy to Other Side's preview");
}

#[test]
fn w9c_ctrl_c_copies_files_from_the_tree_and_not_from_a_text_field() {
    let (mut s, _view, fixed) = w9c_screen();
    s.click("only-left.txt");
    // egui turns Ctrl+C into a copy event; the tree has the keyboard.
    s.input(vec![Event::Copy]);
    s.world.settle(&mut s.app.core);
    s.settle();
    assert_eq!(fixed.file_lists().len(), 1, "Copy ran");
    // The Filters edit has the keyboard: the field keeps it.
    let root = s.root();
    if let Some(w) = s.app.core.window_mut(root) {
        w.focus_filters = true;
    }
    s.settle();
    s.input(vec![Event::Copy]);
    s.input(vec![Event::Paste("x".into())]);
    s.world.settle(&mut s.app.core);
    s.settle();
    assert_eq!(fixed.file_lists().len(), 1, "nothing more was copied");
    assert_eq!(s.dialogs(), 0, "nothing was pasted");
}

#[test]
fn w9c_files_dropped_on_a_pane_ask_to_copy_or_move_them() {
    let (mut s, _view, _) = w9c_screen();
    let outside = s.world.write("src/new.txt", "new\n");
    let over = nth(&mut s, "a.txt", 1).center();
    s.input(vec![Event::PointerMoved(over)]);
    s.time += 0.1;
    let mut input = raw(s.time);
    input.events = vec![Event::PointerMoved(over)];
    input.dropped_files = vec![std::sync::Arc::new(TestDrop(outside)) as egui::DroppedFileHandle];
    frame(&mut s.app, &s.ctx, input);
    s.world.settle(&mut s.app.core);
    s.settle();
    let root = s.root();
    let side = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<crate::drop_dialog::DropDialog>())
        .and_then(|d| d.into.as_ref().map(|i| i.side));
    assert_eq!(
        side,
        Some(hikasa_core::Side::Right),
        "the right pane's prompt"
    );
}

/// A drop whose only pointer position is older than the drag (winit sends none while another program holds it) asks
/// for the pane rather than trusting that position (W9c review F4); Shift held at the drop shows the move first (F8).
#[test]
fn w9c_a_drop_without_a_fresh_position_asks_for_the_pane() {
    let (mut s, _view, _) = w9c_screen();
    let outside = s.world.write("src/new.txt", "new\n");
    let over = nth(&mut s, "a.txt", 1).center();
    // The pointer was over the right pane before the drag began.
    s.input(vec![Event::PointerMoved(over)]);
    s.input(Vec::new());
    // The drag enters the window: files hovered, no position.
    s.time += 0.1;
    let mut input = raw(s.time);
    input.hovered_files = vec![egui::HoveredFile {
        path: Some(outside.clone()),
        mime: String::new(),
    }];
    frame(&mut s.app, &s.ctx, input);
    // Dropped, Shift held, still no position.
    s.time += 0.1;
    let mut input = raw(s.time);
    input.events = vec![Event::ModifiersChanged(egui::Modifiers::SHIFT)];
    input.dropped_files = vec![std::sync::Arc::new(TestDrop(outside)) as egui::DroppedFileHandle];
    frame(&mut s.app, &s.ctx, input);
    s.world.settle(&mut s.app.core);
    s.settle();
    let root = s.root();
    let d = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<crate::drop_dialog::DropDialog>())
        .expect("the prompt");
    assert!(d.sides.is_some(), "it asks for the pane");
    assert!(d.show_move, "Shift shows the move first");
}

/// egui's clipboard events as the catalogue's keys (review F3, F15): Copy, Cut (with Shift: Shift+Del), Paste and an
/// image's paste; a Ctrl+V release is Ctrl+V only when no paste came since its press.
#[test]
fn w9c_clipboard_events_and_a_lone_ctrl_v_release() {
    use egui::{Key, Modifiers};
    let release = Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    let ctrl_v = Some((Key::V, Modifiers::COMMAND));
    let mut seen = false;
    assert_eq!(super::clipboard_key(&release, false, &mut seen), ctrl_v);
    assert_eq!(
        super::clipboard_key(&Event::Paste("x".into()), false, &mut seen),
        ctrl_v
    );
    assert!(seen);
    assert_eq!(super::clipboard_key(&release, false, &mut seen), None);
    assert!(!seen);
    let image = Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled(
        [1, 1],
        egui::Color32::BLACK,
    )));
    assert_eq!(super::clipboard_key(&image, false, &mut seen), ctrl_v);
    assert_eq!(super::clipboard_key(&release, false, &mut seen), None);
    // A press that reached the window is dispatched as it is; its release is not a second paste.
    let press = Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    assert_eq!(super::clipboard_key(&press, false, &mut seen), None);
    assert_eq!(super::clipboard_key(&release, false, &mut seen), None);
    // A release with Shift too, or without Ctrl, is no paste.
    let other = Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    assert_eq!(super::clipboard_key(&other, false, &mut seen), None);
    // Losing the keyboard forgets a paste.
    seen = true;
    assert_eq!(
        super::clipboard_key(&Event::WindowFocused(false), false, &mut seen),
        None
    );
    assert!(!seen);
    assert_eq!(
        super::clipboard_key(&Event::Copy, false, &mut seen),
        Some((Key::C, Modifiers::COMMAND))
    );
    assert_eq!(
        super::clipboard_key(&Event::Cut, false, &mut seen),
        Some((Key::X, Modifiers::COMMAND))
    );
    assert_eq!(
        super::clipboard_key(&Event::Cut, true, &mut seen),
        Some((Key::Delete, Modifiers::SHIFT))
    );
}

/// Ctrl+V with only files on the clipboard: egui-winit sends nothing but the key's release, and that pastes (review
/// F3); a paste event and its release paste once.
#[test]
fn w9c_ctrl_v_with_files_only_pastes_on_its_release() {
    let (mut s, _view, fixed) = w9c_screen();
    let outside = s.world.write("src/new.txt", "new\n");
    fixed.set_files(vec![outside]);
    s.click("only-left.txt");
    let release = Event::Key {
        key: egui::Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    s.input(vec![
        Event::ModifiersChanged(egui::Modifiers::COMMAND),
        release.clone(),
    ]);
    s.world.settle(&mut s.app.core);
    s.settle();
    assert_eq!(s.dialogs(), 1, "the drop's prompt");
    let root = s.root();
    s.app.core.dialog_button(root, ButtonRole::Cancel);
    s.world.settle(&mut s.app.core);
    s.settle();
    let reads = fixed.reads();
    s.input(vec![Event::Paste("text".into())]);
    s.input(vec![release]);
    s.world.settle(&mut s.app.core);
    s.settle();
    assert_eq!(fixed.reads(), reads + 1, "one paste");
}

/// Shift+Del that egui-winit makes a cut of (Windows) is Delete Permanently (review F15).
#[test]
fn w9c_a_cut_with_shift_is_delete_permanently() {
    let (mut s, _view, _) = w9c_screen();
    s.click("only-left.txt");
    s.input(vec![
        Event::ModifiersChanged(egui::Modifiers::SHIFT),
        Event::Cut,
    ]);
    s.input(vec![Event::ModifiersChanged(egui::Modifiers::NONE)]);
    s.world.settle(&mut s.app.core);
    s.settle();
    let root = s.root();
    let asked = s
        .app
        .core
        .window_mut(root)
        .and_then(|w| w.dialogs.find_mut::<crate::op_dialogs::DeleteDialog>())
        .map(|d| d.permanent());
    assert_eq!(asked, Some(true), "Delete Permanently's dialog");
}

/// Shift+F10 and the Menu key open the focus pane's menu for the current row (review F16).
#[test]
fn w9c_the_keyboard_opens_the_pane_menu() {
    for key in [egui::Key::F10, egui::Key::Menu] {
        let (mut s, _view, _) = w9c_screen();
        s.click("only-left.txt");
        let shift = if key == egui::Key::F10 {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        let drawn_before: Vec<String> = s.drawn().into_iter().map(|d| d.text).collect();
        assert!(
            !drawn_before.iter().any(|t| t == "Copy Filename"),
            "{key:?}"
        );
        s.input(vec![
            Event::ModifiersChanged(shift),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: shift,
            },
        ]);
        s.input(vec![
            Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: shift,
            },
            Event::ModifiersChanged(egui::Modifiers::NONE),
        ]);
        let drawn: Vec<String> = s.drawn().into_iter().map(|d| d.text).collect();
        for entry in ["Copy to Other Side", "Copy Filename", "Paste"] {
            assert!(
                drawn.iter().any(|t| t == entry),
                "{key:?} {entry}: {drawn:?}"
            );
        }
    }
}

/// A path dropped from the OS (the integration's file handle).
#[derive(Debug)]
struct TestDrop(std::path::PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
