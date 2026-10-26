//! The egui layer (ARCH-GUI-001): one native window with the menu bar, tabs, toolbar and status bar generated
//! from the command catalogue, the Home form, and the custom-painted folder tree and text panes. It renders the
//! [`AppCore`] every frame and turns input into catalogue commands; it decides nothing itself (ARCH-L3).
//!
//! The window repaints on demand: the mailbox's notifier is `Context::request_repaint`, so an IPC request or a
//! job's news wakes it; while a job runs it also polls at a low rate (the status line shows progress).

pub mod folder;
pub mod home;
pub mod keys;
pub mod scroll;
pub mod text;
pub mod theme;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use egui::{
    Align, AtomExt as _, Button, ComboBox, Frame, Layout, Margin, RichText, Ui, ViewportCommand,
    vec2,
};
use hikasa_session::HIKASA_VERSION;
use hikasa_session::ipc::{CloseReason, ViewId};

use crate::app::{AppCore, Effect, UiMsg, ViewModel};
use crate::bench::BenchState;
use crate::commands::{
    self, Command, CommandId, Ctx, GROUP_FILTER, GROUP_FOLDER_MODE, GROUP_TOGGLES, KeyOrigin, Menu,
    Scope,
};
use crate::mailbox::Mailbox;
use crate::startup::Deferred;
use crate::ui::folder::FolderUi;
use crate::ui::text::TextUi;
use crate::ui::theme::Palette;

/// How often the window looks at running jobs when nothing else wakes it.
const BUSY_POLL: Duration = Duration::from_millis(100);
/// How soon the window looks again when a job has ended but its result is not published yet.
const SETTLE_POLL: Duration = Duration::from_millis(1);
/// Longest tab caption (chars).
const TAB_CAPTION: usize = 40;

/// Something the chrome asked for while it was drawn (done after drawing).
enum Action {
    Run(CommandId),
    Activate(usize),
    CloseTab(usize),
    Dismiss,
}

/// The eframe application.
pub struct HikasaApp {
    core: AppCore,
    mailbox: Mailbox<UiMsg>,
    texts: HashMap<ViewId, TextUi>,
    folders: HashMap<ViewId, FolderUi>,
    bench: Option<BenchState>,
    /// First-run work started after the first frame (`startup`).
    deferred: Deferred,
    hint: Option<&'static str>,
    closing: bool,
}

impl HikasaApp {
    /// The application for the window `cc`; the mailbox wakes the window from now on. `deferred` starts once the
    /// first frame has been drawn.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core: AppCore,
        mailbox: Mailbox<UiMsg>,
        mut bench: Option<BenchState>,
        deferred: Deferred,
    ) -> HikasaApp {
        let ctx = cc.egui_ctx.clone();
        mailbox.set_notifier(Arc::new(move || ctx.request_repaint()));
        // Ctrl+plus/minus/0 are the text views' font commands, not egui's zoom.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        if let Some(b) = bench.as_mut() {
            let renderer = cc.wgpu_render_state.as_ref().map(|rs| {
                let info = rs.adapter.get_info();
                serde_json::json!({
                    "backend": format!("{:?}", info.backend),
                    "adapter": info.name,
                    "driver": info.driver,
                    "device_type": format!("{:?}", info.device_type),
                })
            });
            b.set_renderer(renderer.unwrap_or(serde_json::Value::Null));
        }
        HikasaApp {
            core,
            mailbox,
            texts: HashMap::new(),
            folders: HashMap::new(),
            bench,
            deferred,
            hint: None,
            closing: false,
        }
    }

    /// Applies the core's effects.
    fn apply(&mut self, ctx: &egui::Context, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::TextDirty(view) => self.texts.entry(view).or_default().mark_dirty(),
                Effect::FolderScroll(view, row) => {
                    self.folders.entry(view).or_default().scroll_to(row);
                }
                Effect::Exit => {
                    if !self.closing {
                        self.closing = true;
                        if let Some(report) =
                            self.bench.as_mut().and_then(|b| b.final_report(&self.core))
                        {
                            println!("{report}");
                        }
                    }
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint();
    }

    fn act(&mut self, ctx: &egui::Context, action: Action) {
        let effects = match action {
            Action::Run(id) => self.core.execute(id),
            Action::Activate(index) => {
                self.core.activate(index);
                Vec::new()
            }
            Action::CloseTab(index) => {
                self.core.close_tab(index, CloseReason::User);
                self.core.dirty_effects()
            }
            Action::Dismiss => {
                self.core.dismiss_message();
                Vec::new()
            }
        };
        self.apply(ctx, effects);
    }

    /// Catalogue shortcuts handled at the window root (everything but the focus-bound keys, which the folder
    /// tree takes while it has the focus, UI-003). The keys are removed from the input, so no widget sees them.
    fn shortcuts(&self, ctx: &egui::Context) -> Vec<Action> {
        let scope = self.core.scope();
        let state = self.core.ctx();
        let mut run = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    return true;
                };
                let Some(shortcut) = keys::shortcut_of(*key, *modifiers) else {
                    return true;
                };
                match commands::shortcut_command(scope, shortcut, KeyOrigin::Window) {
                    Some(c) if c.enabled_when.holds(&state) => {
                        run.push(Action::Run(c.id));
                        false
                    }
                    _ => true,
                }
            });
        });
        run
    }

    fn menu_bar(&mut self, ui: &mut Ui, state: &Ctx, actions: &mut Vec<Action>) {
        let scope = self.core.scope();
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in Menu::ORDER {
                let items = commands::menu_items(scope, menu);
                if items.is_empty() {
                    continue;
                }
                ui.menu_button(menu.caption(), |ui| {
                    ui.set_min_width(300.0);
                    let mut group = None;
                    for c in items {
                        if group.is_some_and(|g| g != (c.scope, c.group)) {
                            ui.separator();
                        }
                        group = Some((c.scope, c.group));
                        // Checkable commands get a fixed-width mark column, so their captions line up.
                        let mut button = match self.core.checked(c.id) {
                            Some(on) => Button::new((
                                (if on { "✔" } else { "" }).atom_size(vec2(16.0, 0.0)),
                                c.caption,
                            )),
                            None => Button::new(c.caption),
                        };
                        if let Some(s) = c.shortcut {
                            button = button.shortcut_text(s.to_string());
                        }
                        let response = ui
                            .add_enabled(c.enabled_when.holds(state), button)
                            .on_hover_text(c.hint);
                        if response.hovered() {
                            self.hint = Some(c.hint);
                        }
                        if response.clicked() {
                            actions.push(Action::Run(c.id));
                        }
                    }
                });
            }
        });
    }

    fn tabs(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, tab) in self.core.tabs().iter().enumerate() {
                let caption = crate::shared::shown_name(&tab.caption());
                let short: String = if caption.chars().count() > TAB_CAPTION {
                    let mut s: String = caption.chars().take(TAB_CAPTION - 1).collect();
                    s.push('…');
                    s
                } else {
                    caption.clone()
                };
                let active = i == self.core.active();
                let r = ui
                    .add(Button::selectable(active, short))
                    .on_hover_text(caption);
                if r.clicked() {
                    actions.push(Action::Activate(i));
                }
                if r.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                }
                if ui
                    .add(Button::new(RichText::new("×").small()).frame(false))
                    .on_hover_text("Close Tab (Ctrl+W)")
                    .clicked()
                {
                    actions.push(Action::CloseTab(i));
                }
                ui.add_space(6.0);
            }
            if ui
                .add(Button::new("+").frame(false))
                .on_hover_text("New Tab (Ctrl+T)")
                .clicked()
            {
                actions.push(Action::Run(CommandId::TabNew));
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, state: &Ctx, actions: &mut Vec<Action>) {
        let scope = self.core.scope();
        let tools = commands::toolbar(scope);
        ui.horizontal_wrapped(|ui| {
            for c in tools.iter().filter(|c| c.group == 0) {
                let label = c.toolbar.unwrap_or(c.caption);
                let r = ui
                    .add_enabled(c.enabled_when.holds(state), Button::new(label))
                    .on_hover_text(tooltip(c))
                    .on_disabled_hover_text(tooltip(c));
                if r.hovered() {
                    self.hint = Some(c.hint);
                }
                if r.clicked() {
                    actions.push(Action::Run(c.id));
                }
            }
            ui.separator();
            let presets: Vec<&'static Command> = commands::commands_for(scope)
                .filter(|c| c.group == GROUP_FILTER)
                .collect();
            if !presets.is_empty() {
                let current = presets
                    .iter()
                    .find(|c| self.core.checked(c.id) == Some(true))
                    .map_or("Custom filter", |c| c.caption);
                ComboBox::from_id_salt("hk-filter")
                    .selected_text(current)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for c in &presets {
                            let on = self.core.checked(c.id) == Some(true);
                            if ui
                                .selectable_label(on, c.caption)
                                .on_hover_text(c.hint)
                                .clicked()
                            {
                                actions.push(Action::Run(c.id));
                            }
                        }
                    })
                    .response
                    .on_hover_text("Display filter");
            }
            for c in tools.iter().filter(|c| c.group == GROUP_TOGGLES) {
                let on = self.core.checked(c.id) == Some(true);
                let label = c.toolbar.unwrap_or(c.caption);
                let r = ui
                    .add(Button::selectable(on, label).min_size(vec2(26.0, 0.0)))
                    .on_hover_text(c.caption);
                if r.hovered() {
                    self.hint = Some(c.hint);
                }
                if r.clicked() {
                    actions.push(Action::Run(c.id));
                }
            }
            let modes: Vec<&'static Command> = commands::commands_for(scope)
                .filter(|c| c.group == GROUP_FOLDER_MODE)
                .collect();
            if !modes.is_empty() {
                ui.separator();
                let current = modes
                    .iter()
                    .find(|c| self.core.checked(c.id) == Some(true))
                    .map_or("", |c| c.caption);
                ComboBox::from_id_salt("hk-folder-mode")
                    .selected_text(current)
                    .width(250.0)
                    .show_ui(ui, |ui| {
                        for c in &modes {
                            let on = self.core.checked(c.id) == Some(true);
                            if ui
                                .selectable_label(on, c.caption)
                                .on_hover_text(c.hint)
                                .clicked()
                            {
                                actions.push(Action::Run(c.id));
                            }
                        }
                    })
                    .response
                    .on_hover_text("Folders");
            }
            if let Some(mut on) = self.core.checked(CommandId::InsigIgnored) {
                ui.separator();
                if ui
                    .checkbox(&mut on, "Ignore unimportant")
                    .on_hover_text("Treats unimportant differences as same.")
                    .changed()
                {
                    actions.push(Action::Run(CommandId::InsigIgnored));
                }
            }
        });
    }

    /// Forgets the UI state of views that are gone.
    fn prune(&mut self) {
        let open: Vec<ViewId> = self.core.tabs().iter().map(|t| t.id).collect();
        self.texts.retain(|v, _| open.contains(v));
        self.folders.retain(|v, _| open.contains(v));
    }
}

fn tooltip(c: &Command) -> String {
    match c.shortcut {
        Some(s) => format!("{} ({s}) — {}", c.caption, c.hint),
        None => format!("{} — {}", c.caption, c.hint),
    }
}

/// The left and right parts of the status bar for the active view.
enum ViewStatus {
    Home,
    Folder(crate::folder_view::FolderStatus),
    Text(crate::text_view::TextStatus),
}

impl eframe::App for HikasaApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(b) = self.bench.as_mut() {
            b.begin_frame(frame.info().cpu_usage);
        }
        let mut effects = Vec::new();
        for msg in self.mailbox.drain() {
            effects.extend(self.core.handle(msg));
        }
        // Jobs and the live folder models are polled once per frame (`JobHandle::try_result`, §1.10).
        effects.extend(self.core.handle(UiMsg::Tick));
        if !effects.is_empty() {
            self.apply(ctx, effects);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            // The window's close button: every view reports its outcome (D-23), then the window closes.
            let effects = self.core.shutdown();
            self.apply(ctx, effects);
        }
        if self.core.settling() {
            // A job woke the window before its result was published (microseconds): look again at once.
            ctx.request_repaint_after(SETTLE_POLL);
        } else if self.core.busy() {
            ctx.request_repaint_after(BUSY_POLL);
        }
        if let Some(b) = self.bench.as_mut() {
            b.drive(&self.core, &mut self.texts, &mut self.folders);
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let pal = *Palette::of(&ctx.global_style().visuals);
        let mut actions = self.shortcuts(&ctx);
        self.hint = None;
        let state = self.core.ctx();
        let scope = self.core.scope();

        egui::Panel::top("hk-menu")
            .frame(
                Frame::NONE
                    .fill(pal.chrome)
                    .inner_margin(Margin::symmetric(6, 2)),
            )
            .show(ui, |ui| self.menu_bar(ui, &state, &mut actions));
        egui::Panel::top("hk-tabs")
            .frame(
                Frame::NONE
                    .fill(pal.chrome)
                    .inner_margin(Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| self.tabs(ui, &mut actions));
        if scope != Scope::Home {
            egui::Panel::top("hk-toolbar")
                .frame(
                    Frame::NONE
                        .fill(pal.bg)
                        .inner_margin(Margin::symmetric(8, 5)),
                )
                .show(ui, |ui| self.toolbar(ui, &state, &mut actions));
        }

        let status = match self.core.active_tab().map(|t| &t.model) {
            Some(ViewModel::Folder(m)) => ViewStatus::Folder(m.status()),
            Some(ViewModel::Text(m)) => ViewStatus::Text(m.status()),
            _ => ViewStatus::Home,
        };
        let (left, right) = match &status {
            ViewStatus::Home => (
                "Compare two folders or two files.".to_owned(),
                format!("Hikasa {HIKASA_VERSION}"),
            ),
            ViewStatus::Folder(s) => (s.state.clone(), s.detail.clone()),
            ViewStatus::Text(s) => (s.state.clone(), s.detail.clone()),
        };
        let left = self.hint.map_or(left, str::to_owned);
        egui::Panel::bottom("hk-status")
            .frame(
                Frame::NONE
                    .fill(pal.chrome)
                    .inner_margin(Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let small = |t: &str| RichText::new(t).size(12.0).color(pal.muted);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(egui::Label::new(small(&right)).truncate());
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(small(&left)).truncate());
                        });
                    });
                });
            });
        if let Some(message) = self.core.message() {
            let message = message.to_owned();
            egui::Panel::bottom("hk-message")
                .frame(
                    Frame::NONE
                        .fill(pal.select)
                        .inner_margin(Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(message).color(pal.fg));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("OK").clicked() {
                                actions.push(Action::Dismiss);
                            }
                        });
                    });
                });
        }

        let background = if scope == Scope::Home {
            pal.chrome
        } else {
            pal.bg
        };
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(background))
            .show(ui, |ui| {
                let Some(tab) = self.core.active_tab_mut() else {
                    return;
                };
                let view = tab.id;
                match (&mut tab.model, &status) {
                    (ViewModel::Home(home), _) => {
                        if let Some(id) = home::show(ui, home, &pal) {
                            actions.push(Action::Run(id));
                        }
                    }
                    (ViewModel::Folder(m), ViewStatus::Folder(s)) => {
                        let st = self.folders.entry(view).or_default();
                        let enabled = |id: CommandId| {
                            commands::lookup(Scope::Folder, id)
                                .is_some_and(|c| c.enabled_when.holds(&state))
                        };
                        for id in folder::show(ui, view, m, st, s, &enabled, &pal) {
                            actions.push(Action::Run(id));
                        }
                    }
                    (ViewModel::Text(m), ViewStatus::Text(s)) => {
                        let st = self.texts.entry(view).or_default();
                        text::show(ui, view, m, st, s, &pal);
                    }
                    _ => {}
                }
            });

        let acted = !actions.is_empty();
        for action in actions {
            self.act(&ctx, action);
        }
        // Effects the views raised while drawing (a selection that moved, a gap bar that opened).
        let effects = self.core.dirty_effects();
        if !effects.is_empty() || acted {
            self.apply(&ctx, effects);
        }
        self.prune();
        if self.deferred.pending() {
            // The first frame is laid out: the first-run work runs now, on its own thread (never awaited).
            let _detached = self.deferred.start();
        }
        if let Some(b) = self.bench.as_mut() {
            b.set_window(serde_json::json!({
                "size_points": [ctx.content_rect().width(), ctx.content_rect().height()],
                "pixels_per_point": ctx.pixels_per_point(),
            }));
            b.end_frame(&self.texts);
            if b.finished() && !self.closing {
                // Measure while the view is still open (memory with its comparison, the view's facts).
                if let Some(report) = b.final_report(&self.core) {
                    println!("{report}");
                }
                let effects = self.core.shutdown();
                self.apply(&ctx, effects);
            }
        }
    }

    fn on_exit(&mut self) {
        // The window went away without a close request we saw (e.g. the session ended): views still report.
        if !self.closing {
            self.closing = true;
            let _ = self.core.shutdown();
        }
    }
}
