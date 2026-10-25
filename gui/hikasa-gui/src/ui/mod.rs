//! The Dioxus layer (ARCH-GUI-001): the window chrome (menu bar, tabs, toolbar, status bar) generated from the
//! command catalogue, the Home form, the folder tree and the text canvas host. It renders snapshots of the
//! [`AppCore`] and turns events into catalogue commands; it decides nothing itself.

mod folder;
mod home;
mod text;

use std::sync::Arc;
use std::time::{Duration, Instant};

use dioxus::desktop::{AssetRequest, RequestAsyncResponder, use_asset_handler};
use dioxus::prelude::*;
use parking_lot::Mutex;

use crate::app::{AppCore, Effect, UiMsg, ViewModel};
use crate::bench::BenchState;
use crate::commands::{
    self, Command, CommandId, GROUP_FILTER, GROUP_FOLDER_MODE, GROUP_TOGGLES, KeyOrigin, Menu,
    Scope, Shortcut,
};
use crate::mailbox::{Mailbox, Ticker};

/// Frame interval of the job ticker.
const FRAME: Duration = Duration::from_millis(16);

/// What `main` hands to the root component (taken once).
pub struct StartupData {
    /// The application core.
    pub app: AppCore,
    /// The UI mailbox (IPC handler, jobs, the window's close button post to it).
    pub mailbox: Mailbox<UiMsg>,
    /// Benchmark mode.
    pub bench: Option<BenchState>,
}

/// The startup data as a context value (`Clone + Send + Sync` for the launcher).
#[derive(Clone)]
pub struct Startup(pub Arc<Mutex<Option<StartupData>>>);

/// Handles shared by every component (all `Copy`).
#[derive(Clone, Copy)]
pub struct Ui {
    /// The core (not reactive: `rev` re-renders).
    pub app: CopyValue<AppCore>,
    /// Bumped after every change; the root reads it.
    pub rev: Signal<u64>,
    /// The open menu.
    pub menu: Signal<Option<Menu>>,
    /// Benchmark state.
    pub bench: CopyValue<Option<BenchState>>,
}

impl Ui {
    /// Runs `f` on the core, applies its effects and re-renders.
    pub fn act(mut self, f: impl FnOnce(&mut AppCore) -> Vec<Effect>) {
        let effects = {
            let mut app = self.app.write();
            f(&mut app)
        };
        self.apply(effects);
        self.rev += 1;
    }

    /// Executes a catalogue command.
    pub fn command(self, id: CommandId) {
        self.act(|app| app.execute(id));
    }

    /// Applies effects.
    pub fn apply(self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::TextDirty(view) => {
                    let _ = document::eval(&format!(
                        "window.hikasa && window.hikasa.textDirty({})",
                        view.0
                    ));
                }
                Effect::FolderScroll(view, index) => {
                    let _ = document::eval(&format!(
                        "window.hikasa && window.hikasa.folderScrollTo({}, {index}, {})",
                        view.0,
                        crate::folder_view::ROW_HEIGHT
                    ));
                }
                Effect::Exit => {
                    let mut bench = self.bench;
                    let report = bench.write().take().and_then(|b| b.final_report());
                    if let Some(report) = report {
                        println!("{report}");
                    }
                    std::process::exit(0);
                }
            }
        }
    }
}

/// The root component.
pub fn root() -> Element {
    let startup = use_context::<Startup>();
    let ui = use_hook(|| {
        let data = startup.0.lock().take();
        let (app, mailbox, bench) = match data {
            Some(d) => (d.app, d.mailbox, d.bench),
            None => {
                // A second root (never happens): nothing to run.
                return None;
            }
        };
        let ui = Ui {
            app: CopyValue::new(app),
            rev: Signal::new(0),
            menu: Signal::new(None),
            bench: CopyValue::new(bench),
        };
        spawn(event_loop(ui, mailbox));
        Some(ui)
    });
    let Some(ui) = ui else {
        return rsx! {};
    };
    use_context_provider(|| ui);
    use_asset_handler(
        "hk",
        move |request: AssetRequest, responder: RequestAsyncResponder| {
            crate::rpc::handle(ui, &request, responder);
        },
    );
    let render_began = Instant::now();
    let _rev = (ui.rev)();
    let element = render_app(ui);
    let mut bench = ui.bench;
    if let Some(bench) = bench.write().as_mut() {
        bench.record_render(render_began.elapsed());
    }
    element
}

/// The UI thread's loop: messages from the IPC handler, the jobs and the ticker.
async fn event_loop(ui: Ui, mailbox: Mailbox<UiMsg>) {
    let ticks = mailbox.clone();
    let ticker = Ticker::start(FRAME, move || ticks.post(UiMsg::Tick));
    loop {
        let msg = mailbox.next().await;
        let began = Instant::now();
        let tick = matches!(msg, UiMsg::Tick);
        let mut app = ui.app;
        let before = app.read().revision();
        let mut effects = app.write().handle(msg);
        // Handle whatever else is queued in the same frame.
        for msg in mailbox.drain() {
            let tick_again = matches!(msg, UiMsg::Tick);
            effects.extend(app.write().handle(msg));
            if tick_again {
                ticker.ticked();
            }
        }
        if tick {
            ticker.ticked();
        }
        let (after, busy) = {
            let app = app.read();
            (app.revision(), app.busy())
        };
        ticker.set_busy(busy);
        let changed = after != before;
        let mut bench_ui = ui.bench;
        if let Some(bench) = bench_ui.write().as_mut() {
            bench.record_tick(began.elapsed(), changed);
        }
        ui.apply(effects);
        if changed {
            let mut rev = ui.rev;
            rev += 1;
        }
        crate::bench::after_frame(ui);
    }
}

fn render_app(ui: Ui) -> Element {
    let app = ui.app.read();
    let scope = app.scope();
    let ctx = app.ctx();
    // Reading the signal subscribes the root: opening a menu re-renders.
    let open_menu = (ui.menu)();
    let tabs: Vec<(usize, String, bool)> = app
        .tabs()
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.caption(), i == app.active()))
        .collect();
    let message = app.message().map(str::to_owned);
    let content = match app.active_tab().map(|t| (&t.model, t.id)) {
        Some((ViewModel::Home(home), _)) => {
            let home = home.clone();
            rsx! { home::HomePane { home } }
        }
        Some((ViewModel::Folder(m), id)) => {
            let window = m.window();
            let status = m.status();
            rsx! { folder::FolderPane { key: "{id.0}", view: id.0, window, status } }
        }
        Some((ViewModel::Text(m), id)) => {
            let status = m.status();
            rsx! { text::TextPane { key: "{id.0}", view: id.0, status } }
        }
        None => rsx! {},
    };
    drop(app);
    frame(ui, scope, ctx, open_menu, tabs, message, content)
}

/// The window frame around the active view.
fn frame(
    ui: Ui,
    scope: Scope,
    ctx: commands::Ctx,
    open_menu: Option<Menu>,
    tabs: Vec<(usize, String, bool)>,
    message: Option<String>,
    content: Element,
) -> Element {
    let checked = |id: CommandId| ui.app.read().checked(id);
    let toolbar: Vec<&'static Command> = commands::toolbar(scope);
    let plain: Vec<&'static Command> = toolbar.iter().copied().filter(|c| c.group == 0).collect();
    let toggles: Vec<&'static Command> = toolbar
        .iter()
        .copied()
        .filter(|c| c.group == GROUP_TOGGLES)
        .collect();
    let presets: Vec<&'static Command> = commands::commands_for(scope)
        .filter(|c| c.group == GROUP_FILTER)
        .collect();
    let modes: Vec<&'static Command> = commands::commands_for(scope)
        .filter(|c| c.group == GROUP_FOLDER_MODE)
        .collect();
    let current_preset = presets.iter().position(|c| checked(c.id) == Some(true));
    let current_mode = modes.iter().position(|c| checked(c.id) == Some(true));
    let iu = checked(CommandId::InsigIgnored);
    rsx! {
        div {
            class: "hk-app",
            tabindex: "0",
            // Keyboard shortcuts work before the first click.
            onmounted: move |evt: MountedEvent| async move {
                let _ = evt.set_focus(true).await;
            },
            // Shortcuts that focused controls do not use themselves (plain Enter and arrows stay with the
            // focused button, list or field; the tree takes them itself).
            onkeydown: move |evt: KeyboardEvent| {
                key_command(ui, &evt, KeyOrigin::Window);
            },
            onclick: move |_| {
                let mut menu = ui.menu;
                if menu.peek().is_some() {
                    menu.set(None);
                }
            },
            // Menu bar
            div { class: "hk-menubar",
                for m in Menu::ORDER {
                    if !commands::menu_items(scope, m).is_empty() {
                        div { class: "hk-menu",
                            button {
                                class: if open_menu == Some(m) { "hk-menu-title hk-open" } else { "hk-menu-title" },
                                onclick: move |evt: MouseEvent| {
                                    evt.stop_propagation();
                                    let mut menu = ui.menu;
                                    let next = if *menu.peek() == Some(m) { None } else { Some(m) };
                                    menu.set(next);
                                },
                                "{m.caption()}"
                            }
                            if open_menu == Some(m) {
                                div { class: "hk-dropdown",
                                    for c in commands::menu_items(scope, m) {
                                        {menu_item(ui, c, c.enabled_when.holds(&ctx), checked(c.id))}
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Tabs
            div { class: "hk-tabs",
                for (i, caption, active) in tabs {
                    div {
                        class: if active { "hk-tab hk-active" } else { "hk-tab" },
                        title: "{caption}",
                        onclick: move |_| ui.act(|app| { app.activate(i); Vec::new() }),
                        span { class: "hk-tab-caption", "{caption}" }
                        button {
                            class: "hk-tab-close",
                            title: "Close Tab",
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                ui.act(|app| {
                                    app.close_tab(i, hikasa_session::ipc::CloseReason::User);
                                    app.dirty_effects()
                                });
                            },
                            "×"
                        }
                    }
                }
                button {
                    class: "hk-tab-new",
                    title: "New Tab (Ctrl+T)",
                    onclick: move |_| ui.command(CommandId::TabNew),
                    "+"
                }
            }
            // Toolbar
            if scope != Scope::Home {
                div { class: "hk-toolbar",
                    for c in plain {
                        button {
                            class: "hk-tool",
                            disabled: !c.enabled_when.holds(&ctx),
                            title: "{tooltip(c)}",
                            "data-command": c.id.name(),
                            onclick: move |_| ui.command(c.id),
                            "{c.toolbar.unwrap_or(c.caption)}"
                        }
                    }
                    if !presets.is_empty() {
                        select {
                            class: "hk-select",
                            title: "Display filter",
                            onchange: move |evt: FormEvent| {
                                let presets: Vec<&'static Command> = commands::commands_for(scope)
                                    .filter(|c| c.group == GROUP_FILTER)
                                    .collect();
                                if let Some(c) = evt.value().parse::<usize>().ok().and_then(|i| presets.get(i)) {
                                    ui.command(c.id);
                                }
                            },
                            if current_preset.is_none() {
                                option { value: "", selected: true, "Custom filter" }
                            }
                            for (i, c) in presets.iter().enumerate() {
                                option { value: "{i}", selected: current_preset == Some(i), "{c.caption}" }
                            }
                        }
                    }
                    div { class: "hk-toggles",
                        for c in toggles {
                            button {
                                class: if checked(c.id) == Some(true) { "hk-toggle hk-on" } else { "hk-toggle" },
                                title: "{c.caption}",
                                onclick: move |_| ui.command(c.id),
                                "{c.toolbar.unwrap_or(c.caption)}"
                            }
                        }
                    }
                    if !modes.is_empty() {
                        select {
                            class: "hk-select",
                            title: "Folders",
                            onchange: move |evt: FormEvent| {
                                let modes: Vec<&'static Command> = commands::commands_for(scope)
                                    .filter(|c| c.group == GROUP_FOLDER_MODE)
                                    .collect();
                                if let Some(c) = evt.value().parse::<usize>().ok().and_then(|i| modes.get(i)) {
                                    ui.command(c.id);
                                }
                            },
                            for (i, c) in modes.iter().enumerate() {
                                option { value: "{i}", selected: current_mode == Some(i), "{c.caption}" }
                            }
                        }
                    }
                    if let Some(on) = iu {
                        label { class: "hk-check",
                            input {
                                r#type: "checkbox",
                                checked: on,
                                onchange: move |_| ui.command(CommandId::InsigIgnored),
                            }
                            "Ignore unimportant"
                        }
                    }
                }
            }
            div { class: "hk-content", {content} }
            if let Some(text) = message {
                div { class: "hk-message",
                    span { "{text}" }
                    button {
                        class: "hk-tool",
                        onclick: move |_| ui.act(|app| { app.dismiss_message(); Vec::new() }),
                        "OK"
                    }
                }
            }
        }
    }
}

/// Runs the catalogue command of a key event handled at `origin` when it is enabled in the active view;
/// only then the key's default action is prevented (and it does not bubble further). True when a command ran.
pub fn key_command(ui: Ui, evt: &KeyboardEvent, origin: KeyOrigin) -> bool {
    let Some(shortcut) = shortcut_of(&evt.data()) else {
        return false;
    };
    let id = {
        let app = ui.app.read();
        commands::shortcut_command(app.scope(), shortcut, origin)
            .map(|c| c.id)
            .filter(|&id| app.enabled(id))
    };
    match id {
        Some(id) => {
            evt.prevent_default();
            evt.stop_propagation();
            ui.command(id);
            true
        }
        None => false,
    }
}

fn tooltip(c: &Command) -> String {
    match c.shortcut {
        Some(s) => format!("{} ({s}) — {}", c.caption, c.hint),
        None => format!("{} — {}", c.caption, c.hint),
    }
}

fn menu_item(ui: Ui, c: &'static Command, enabled: bool, checked: Option<bool>) -> Element {
    let mark = match checked {
        Some(true) => "✓",
        _ => "",
    };
    let shortcut = c.shortcut.map(|s| s.to_string()).unwrap_or_default();
    rsx! {
        button {
            class: "hk-item",
            disabled: !enabled,
            title: "{c.hint}",
            "data-command": c.id.name(),
            onclick: move |evt: MouseEvent| {
                evt.stop_propagation();
                let mut menu = ui.menu;
                menu.set(None);
                ui.command(c.id);
            },
            span { class: "hk-item-mark", "{mark}" }
            span { class: "hk-item-caption", "{c.caption}" }
            span { class: "hk-item-key", "{shortcut}" }
        }
    }
}

/// The catalogue shortcut of a key event (letters and digits upper-cased; `+`, `=` and `-` with Ctrl also
/// stand for the keypad keys).
pub fn shortcut_of(evt: &KeyboardData) -> Option<Shortcut> {
    use commands::Key as K;
    let mods = evt.modifiers();
    let ctrl = mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META);
    let shift = mods.contains(Modifiers::SHIFT);
    let alt = mods.contains(Modifiers::ALT);
    let key = match evt.code() {
        Code::NumpadAdd => K::NumPlus,
        Code::NumpadSubtract => K::NumMinus,
        Code::Numpad0 => K::NumZero,
        _ => match evt.key() {
            Key::Enter => K::Enter,
            Key::Tab => K::Tab,
            Key::ArrowUp => K::Up,
            Key::ArrowDown => K::Down,
            Key::ArrowLeft => K::Left,
            Key::ArrowRight => K::Right,
            Key::F1 => K::F(1),
            Key::F2 => K::F(2),
            Key::F3 => K::F(3),
            Key::F4 => K::F(4),
            Key::F5 => K::F(5),
            Key::F6 => K::F(6),
            Key::F7 => K::F(7),
            Key::F8 => K::F(8),
            Key::F9 => K::F(9),
            Key::F10 => K::F(10),
            Key::F11 => K::F(11),
            Key::F12 => K::F(12),
            Key::Character(s) => {
                let mut chars = s.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                match c {
                    '+' | '=' if ctrl => K::NumPlus,
                    '-' if ctrl => K::NumMinus,
                    '0' if ctrl => K::NumZero,
                    c if c.is_ascii_alphanumeric() => K::Char(c.to_ascii_uppercase()),
                    _ => return None,
                }
            }
            _ => return None,
        },
    };
    Some(Shortcut {
        ctrl,
        shift: shift && !matches!(key, K::NumPlus),
        alt,
        key,
    })
}
