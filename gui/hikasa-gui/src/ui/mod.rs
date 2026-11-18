//! The egui layer (ARCH-GUI-001; M2 W4a, §1.4.1–§1.4.8): every window of the [`AppCore`] — `windows[0]` in
//! eframe's root viewport, the others as immediate viewports (D-106) — with its menu bar, tab bar, toolbar,
//! status bar, message panel, log panel, help panel, notices and dialogs, generated from the effective command
//! table, plus the Home form and the custom-painted folder tree and text panes. It renders the core every frame
//! and turns input into invocations and model calls; it decides nothing itself (ARCH-L3, T11).
//!
//! The windows repaint on demand: the mailbox's notifier is `Context::request_repaint`, so an IPC request, the
//! watch thread or a job's news wakes them; while a job runs they also poll at a low rate.

pub mod browse;
pub mod dialog;
pub mod find_files;
pub mod find_panel;
pub mod folder;
pub mod folder_tabs;
pub mod fonts;
pub mod formats;
pub mod help;
pub mod hex;
pub mod hex_panels;
pub mod hex_tabs;
pub mod hint;
pub mod home;
pub mod icons;
pub mod keys;
pub mod legend;
pub mod log;
pub mod menus;
pub mod notify;
pub mod op_dialogs;
pub mod options;
pub mod picture;
pub mod quick_start;
pub mod report_dialog;
pub mod script_dialog;
pub mod scroll;
pub mod session_dialog;
pub mod status;
pub mod sync_view;
pub mod table;
pub mod table_dialogs;
pub mod text;
pub mod text_header;
pub mod text_panels;
pub mod text_tabs;
pub mod theme;
pub mod wizards;
pub mod workspace;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use egui::{
    Button, Frame, Margin, RichText, Sense, Ui, ViewportBuilder, ViewportCommand, ViewportId,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::settings::state::WindowState;

use crate::app::{AppCore, Effect, UiMsg};
use crate::bench::BenchState;
use crate::commands::{CommandId, CommandTables, FocusArea, Scope};
use crate::dnd::{DropTarget, FieldId};
use crate::menus::{MenuContext, ToolItem, ToolbarMenuItem, menu_bar, tab_menu, toolbar_items};
use crate::notify::NoticeAction;
use crate::options::theme_page::FontSlot;
use crate::startup::Deferred;
use crate::ui::dialog::DialogInput;
use crate::ui::folder::FolderUi;
use crate::ui::fonts::SlotFonts;
use crate::ui::hint::Hint as _;
use crate::ui::home as ui_home;
use crate::ui::log::LogAction;
use crate::ui::menus::{Hover, ToolbarAction, ToolbarCtx};
use crate::ui::status::{MessageAction, StatusAction};
use crate::ui::text::TextUi;
use crate::ui::theme::Palette;
use crate::watch::WatchLink;
use crate::window::{Invocation, ScreenRect, TabDrop, ViewModel, WindowId, resolve_tab_drop};

/// How often the window looks at running jobs when nothing else wakes it.
const BUSY_POLL: Duration = Duration::from_millis(100);
/// How soon the window looks again when a job has ended but its result is not published yet.
const SETTLE_POLL: Duration = Duration::from_millis(1);
/// Longest tab caption (chars).
const TAB_CAPTION: usize = 40;

/// Something a window asked for while it was drawn (done after drawing it).
enum Action {
    Run(Invocation),
    Activate(usize),
    CloseTab(usize),
    Help(String),
    CloseHelp,
    Dialog(DialogInput),
    Notice(String, NoticeAction),
    Message(ViewId, MessageAction),
    Status(StatusAction),
    ToolbarMenu(ToolbarMenuItem),
    Log(ViewId, LogAction),
    LogHeight(ViewId, f64),
    MoveTab(usize, Option<WindowId>, Option<usize>, Option<[f32; 2]>),
    /// Paths dropped from the OS; Shift held at the drop (a pane's prompt shows the move first).
    Drop(DropTarget, Vec<std::path::PathBuf>, bool),
    Home(ui_home::HomeAction),
    /// The inline Rename's Enter (M2 W9b, D-245).
    CommitRename,
    /// Rows dragged in the active folder view were released (M2 W9c, D-251).
    RowDrop(crate::dnd::RowDrag, crate::dnd::RowTarget, bool),
    /// The Filters edit: Enter (`None`) or a preset or recent filter chosen (M2 W9c, D-250).
    FiltersEnter(Option<String>),
    /// The Filters edit's *Add To Presets*.
    FiltersAdd,
    /// The Find in Files panel (M2 W11b, D-308).
    FindFiles(find_files::FilesAction),
}

/// The id of a folder view's Filters edit.
pub fn filters_id(view: ViewId) -> egui::Id {
    egui::Id::unique(("hk-filters-edit", view.0))
}

/// The Filters edit (D-250): the text field, Enter applies, Esc reverts; the drop-down of the presets and the recent
/// filters, *Add To Presets*; core's message under it.
fn filters_field(
    ui: &mut Ui,
    view: ViewId,
    edit: &mut crate::filters_edit::FiltersEdit,
    presets: &[(String, String)],
    recent: &[String],
    focus: bool,
    pal: &crate::ui::theme::Palette,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label("Filters");
        let id = filters_id(view);
        let response = ui.add(
            egui::TextEdit::singleline(&mut edit.text)
                .id(id)
                .hint_text("*.* (all files)")
                .desired_width(320.0)
                .font(egui::FontId::monospace(12.0)),
        );
        if focus {
            response.request_focus();
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = Some(Action::FiltersEnter(None));
        } else if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            edit.revert();
        }
        ui.menu_button("▾", |ui| {
            if !presets.is_empty() {
                ui.label(RichText::new("Presets").color(pal.muted));
                for (name, mask) in presets {
                    if ui.button(name).on_hover_text(mask).clicked() {
                        action = Some(Action::FiltersEnter(Some(mask.clone())));
                        ui.close();
                    }
                }
            }
            if !recent.is_empty() {
                ui.separator();
                ui.label(RichText::new("Recent").color(pal.muted));
                for mask in recent {
                    if ui.button(mask).clicked() {
                        action = Some(Action::FiltersEnter(Some(mask.clone())));
                        ui.close();
                    }
                }
            }
            ui.separator();
            if ui.button("Add To Presets…").clicked() {
                action = Some(Action::FiltersAdd);
                ui.close();
            }
        });
        if let Some(error) = &edit.error {
            ui.label(RichText::new(error).color(pal.error));
        }
    });
    action
}

/// Where a window's parts were last drawn (tab drags, D-107).
#[derive(Debug, Clone, Default)]
struct Placement {
    /// The window's outer rectangle in global points, when the platform reports it.
    outer: Option<ScreenRect>,
    /// The inner rectangle's top-left in global points (local → global).
    inner_min: Option<[f32; 2]>,
    /// The tab bar in global points.
    bar: Option<ScreenRect>,
}

/// The eframe application.
pub struct HikasaApp {
    core: AppCore,
    /// The auto-save timer's time (§1.4.9).
    clock: std::time::Instant,
    mailbox: Mailbox,
    texts: HashMap<ViewId, TextUi>,
    /// The *Replace with* text of each window's Find in Files panel.
    files_replace: HashMap<WindowId, String>,
    tables: HashMap<ViewId, table::TableUi>,
    hexes: HashMap<ViewId, hex::HexUi>,
    /// The textures of each picture view (M2 W13b: `picture.rs` owns them).
    pictures: HashMap<ViewId, picture::PictureUi>,
    /// Frames shown of picture views (the window budget's least recently shown order).
    picture_clock: u64,
    folders: HashMap<ViewId, FolderUi>,
    bench: Option<BenchState>,
    /// First-run work started after the first frame (`startup`).
    deferred: Deferred,
    closing: bool,
    palette: Palette,
    /// The family of each font slot.
    slot_fonts: SlotFonts,
    /// What `slot_fonts` becomes when the definitions of [`Effect::ApplyFonts`] are in use.
    next_fonts: Option<SlotFonts>,
    os_dark: Option<bool>,
    hover: Hover,
    placements: HashMap<WindowId, Placement>,
    root_title: String,
    /// The window a tab is dragged from, and the tab.
    drag: Option<(WindowId, usize)>,
    /// The watch thread (D-117).
    watch: Option<std::thread::JoinHandle<()>>,
    /// The viewport builder of each window that is not the root, made when its viewport opens: geometry goes
    /// to the platform only then (or when the model moves the window), never back from what it reported.
    builders: HashMap<WindowId, ViewportBuilder>,
    /// The toolbar items of the tables and scope drawn last (rebuilt when either changes).
    toolbar_cache: Option<(Arc<CommandTables>, Scope, Vec<ToolItem>)>,
    /// Per window: when the drag from another program began and when the pointer last moved (W9c review F4).
    os_drags: HashMap<WindowId, OsDrag>,
    /// Per window: a paste came since the last Ctrl+V release (W9c review F3, [`clipboard_key`]).
    paste_seen: HashMap<WindowId, bool>,
}

/// The catalogue key an egui clipboard event stands for (D-252, §1.9.25 W9c note 8): Copy is Ctrl+C, Cut Ctrl+X (with
/// Shift held, the Shift+Del egui-winit makes a cut of on Windows: Delete Permanently; review F15), Paste and an
/// image's paste Ctrl+V. egui-winit swallows a pressed Ctrl+V and sends a paste only when the clipboard holds text or an
/// image, so with files alone nothing comes but the key's release: a Ctrl/Cmd+V (Windows: Shift+Insert) release with
/// no paste since its press is Ctrl+V (review F3). `paste_seen` keeps, across frames, that a paste came since the last
/// release.
fn clipboard_key(
    event: &egui::Event,
    shift: bool,
    paste_seen: &mut bool,
) -> Option<(egui::Key, egui::Modifiers)> {
    let ctrl_v = (egui::Key::V, egui::Modifiers::COMMAND);
    match event {
        egui::Event::Copy => Some((egui::Key::C, egui::Modifiers::COMMAND)),
        egui::Event::Cut if shift => Some((egui::Key::Delete, egui::Modifiers::SHIFT)),
        egui::Event::Cut => Some((egui::Key::X, egui::Modifiers::COMMAND)),
        egui::Event::Paste(_) | egui::Event::PasteImage(_) => {
            *paste_seen = true;
            Some(ctrl_v)
        }
        egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } if is_paste_key(*key, *modifiers) => {
            if *pressed {
                // The press reached the window itself: it is dispatched as it is.
                *paste_seen = true;
                None
            } else if std::mem::take(paste_seen) {
                None
            } else {
                Some(ctrl_v)
            }
        }
        egui::Event::WindowFocused(false) => {
            *paste_seen = false;
            None
        }
        _ => None,
    }
}

/// Ctrl/Cmd+V, or Shift+Insert on Windows: the keys egui-winit makes a paste of.
fn is_paste_key(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    (key == egui::Key::V && modifiers.command_only())
        || (cfg!(windows) && key == egui::Key::Insert && modifiers.shift && !modifiers.command)
}

/// What a window knows of a drag from another program (W9c review F4): winit sends no pointer position while the
/// other program holds the drag (X11 ignores `XdndPosition`, Windows and macOS give none), so the position egui keeps
/// is the one from before the drag, often over another place. A position counts only when the pointer moved after
/// the drag began (in its first frame at the earliest).
#[derive(Debug, Default, Clone, Copy)]
struct OsDrag {
    /// The input time of the drag's first frame (files hovered or dropped).
    since: Option<f64>,
    /// The input time of the last pointer move or button.
    moved: Option<f64>,
}

impl OsDrag {
    /// One frame: `moved` (a pointer event in it), `hovering` (files over the window), `dropping` (files dropped);
    /// true when the pointer's position is as fresh as the drag.
    fn frame(&mut self, now: f64, moved: bool, hovering: bool, dropping: bool) -> bool {
        if moved {
            self.moved = Some(now);
        }
        if (hovering || dropping) && self.since.is_none() {
            self.since = Some(now);
        }
        let since = self.since.unwrap_or(now);
        let fresh = self.moved.is_some_and(|t| t >= since);
        if !hovering {
            self.since = None;
        }
        fresh
    }
}

type Mailbox = crate::mailbox::Mailbox<UiMsg>;

/// The viewport of a window that is not the root.
fn viewport_of(id: WindowId) -> ViewportId {
    ViewportId::from_hash_of(("hikasa-window", id.0))
}

fn rect_of(r: egui::Rect) -> ScreenRect {
    ScreenRect {
        min: [r.min.x, r.min.y],
        max: [r.max.x, r.max.y],
    }
}

impl HikasaApp {
    /// The application for the window `cc`; the mailbox wakes the window from now on, the watch thread starts
    /// (`link` connects the core). `deferred` starts once the first frame has been drawn.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core: AppCore,
        mailbox: Mailbox,
        mut bench: Option<BenchState>,
        deferred: Deferred,
        watch: Option<(crate::watch::Watcher, WatchLink)>,
    ) -> HikasaApp {
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
        HikasaApp::with_context(&cc.egui_ctx, core, mailbox, bench, deferred, watch)
    }

    /// The application drawn with `ctx` (eframe's, or a headless one in tests).
    pub fn with_context(
        ctx: &egui::Context,
        mut core: AppCore,
        mailbox: Mailbox,
        bench: Option<BenchState>,
        deferred: Deferred,
        watch: Option<(crate::watch::Watcher, WatchLink)>,
    ) -> HikasaApp {
        let notify = ctx.clone();
        mailbox.set_notifier(Arc::new(move || notify.request_repaint()));
        // Ctrl+plus/minus/0 are the text views' font commands, not egui's zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // Hints follow `hint_show_ms` (`ui::hint`): egui's own delay is 0, in the light and the dark style.
        ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        let watch = watch.and_then(|(watcher, link)| {
            core.set_watch(link);
            let mailbox = mailbox.clone();
            watcher
                .spawn(move |events| mailbox.post(UiMsg::Watch(events)))
                .map_err(
                    |e| tracing::warn!(target: "hikasa::gui", "cannot start the watch thread: {e}"),
                )
                .ok()
        });
        let palette = Palette::from_scheme_base(core.scheme(), core.scheme_base());
        set_theme(ctx, &palette, core.scheme());
        HikasaApp {
            core,
            clock: std::time::Instant::now(),
            mailbox,
            texts: HashMap::new(),
            files_replace: HashMap::new(),
            tables: HashMap::new(),
            hexes: HashMap::new(),
            pictures: HashMap::new(),
            picture_clock: 0,
            folders: HashMap::new(),
            bench,
            deferred,
            closing: false,
            palette,
            slot_fonts: SlotFonts::default(),
            next_fonts: None,
            os_dark: None,
            hover: Hover::default(),
            placements: HashMap::new(),
            root_title: String::new(),
            drag: None,
            watch,
            builders: HashMap::new(),
            toolbar_cache: None,
            os_drags: HashMap::new(),
            paste_seen: HashMap::new(),
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
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                }
                Effect::ApplyTheme => {
                    self.palette =
                        Palette::from_scheme_base(self.core.scheme(), self.core.scheme_base());
                    set_theme(ctx, &self.palette, self.core.scheme());
                }
                Effect::ApplyFonts => {
                    ctx.set_fonts(fonts::definitions(self.core.fonts()));
                    // From the next frame: egui takes new definitions at the start of a pass.
                    self.next_fonts = Some(SlotFonts::of(self.core.fonts()));
                }
                Effect::Geometry(window, g) => {
                    let target = if window == self.core.root() {
                        ViewportId::ROOT
                    } else {
                        viewport_of(window)
                    };
                    ctx.send_viewport_cmd_to(
                        target,
                        ViewportCommand::InnerSize(egui::vec2(g.width as f32, g.height as f32)),
                    );
                    if g.x >= 0 && g.y >= 0 {
                        ctx.send_viewport_cmd_to(
                            target,
                            ViewportCommand::OuterPosition(egui::pos2(g.x as f32, g.y as f32)),
                        );
                    }
                    ctx.send_viewport_cmd_to(target, ViewportCommand::Maximized(g.maximized));
                    self.root_title.clear();
                }
                // A window drawn in the root viewport since the root swap gets a new builder if it ever leaves it.
                Effect::CloseViewport(w) => {
                    self.builders.remove(&w);
                }
                Effect::Raise(window) => {
                    let target = if window == self.core.root() {
                        ViewportId::ROOT
                    } else {
                        viewport_of(window)
                    };
                    ctx.send_viewport_cmd_to(target, ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd_to(target, ViewportCommand::Focus);
                }
                Effect::Beep(window) => {
                    let target = if window == self.core.root() {
                        ViewportId::ROOT
                    } else {
                        viewport_of(window)
                    };
                    ctx.send_viewport_cmd_to(
                        target,
                        ViewportCommand::RequestUserAttention(
                            egui::UserAttentionType::Informational,
                        ),
                    );
                    crate::ui::script_dialog::flash(ctx);
                }
                // A picture pane alone in the window (M2 W13b, D-362): the compositor decides under Wayland.
                Effect::FullScreen(window, on) => {
                    let target = if window == self.core.root() {
                        ViewportId::ROOT
                    } else {
                        viewport_of(window)
                    };
                    ctx.send_viewport_cmd_to(target, ViewportCommand::Fullscreen(on));
                }
                Effect::OpenViewport(_) | Effect::ApplyCommands => {}
            }
        }
        ctx.request_repaint();
    }

    fn act(&mut self, ctx: &egui::Context, window: WindowId, action: Action) {
        let effects = match action {
            Action::Run(inv) => self.core.execute(window, inv),
            Action::Activate(index) => {
                self.core.activate(window, index);
                Vec::new()
            }
            Action::CloseTab(index) => {
                self.core.activate(window, index);
                self.core.execute(window, CommandId::CloseTab)
            }
            Action::Help(topic) => {
                self.core.show_help(window, &topic);
                Vec::new()
            }
            Action::CloseHelp => {
                self.core.close_help(window);
                Vec::new()
            }
            Action::CommitRename => {
                self.core.commit_inline_rename(window);
                Vec::new()
            }
            Action::FindFiles(a) => self.core.find_files_action(window, a),
            Action::RowDrop(drag, target, shift) => {
                self.core.drop_rows(window, &drag, target, shift)
            }
            Action::FiltersEnter(mask) => {
                self.core.filters_enter(window, mask.as_deref());
                Vec::new()
            }
            Action::FiltersAdd => {
                self.core.filters_add_to_presets(window);
                Vec::new()
            }
            Action::Dialog(DialogInput::Button(role)) => self.core.dialog_button(window, role),
            Action::Dialog(DialogInput::DoNotAsk(on)) => {
                self.core.set_do_not_ask(window, on);
                Vec::new()
            }
            Action::Dialog(DialogInput::Help(topic)) => {
                self.core.show_help(window, topic);
                Vec::new()
            }
            Action::Dialog(DialogInput::Edited) => self.core.dialog_edited(window),
            Action::Dialog(DialogInput::Browse(input)) => self.core.browse_input(window, input),
            Action::Dialog(DialogInput::BrowseFor(mode, start, target)) => {
                self.core.open_browse(window, mode, start, target);
                Vec::new()
            }
            Action::Notice(key, action) => self.core.notice_action(window, &key, action),
            Action::Message(view, m) => {
                self.core
                    .dismiss_message(view, matches!(m, MessageAction::DoNotShow));
                Vec::new()
            }
            Action::Status(StatusAction::Cancel(view)) => {
                self.core.cancel_jobs(view);
                Vec::new()
            }
            Action::Status(StatusAction::Run(inv)) => self.core.execute(window, inv),
            Action::ToolbarMenu(item) => match item {
                ToolbarMenuItem::Mode(mode, _) => {
                    self.core.set_display_filter_mode(window, mode);
                    Vec::new()
                }
                ToolbarMenuItem::LargeButtons(on) => {
                    self.core.set_large_buttons(window, !on);
                    Vec::new()
                }
                ToolbarMenuItem::HideToolbar => self.core.execute(window, CommandId::ShowToolbar),
                ToolbarMenuItem::Options => {
                    let view = self.core.scope(window).view_name().unwrap_or("home");
                    self.core.open_commands_page(window, view);
                    Vec::new()
                }
            },
            Action::Log(view, LogAction::Clear) => {
                let _ = view;
                self.core.clear_log();
                Vec::new()
            }
            Action::Log(_, LogAction::Hide) => self.core.execute(window, CommandId::ShowLog),
            Action::LogHeight(view, h) => {
                if let Some(ViewModel::Folder(m)) =
                    self.core.tab_mut(view).and_then(|t| t.model_mut())
                {
                    m.set_log_height(h);
                }
                Vec::new()
            }
            Action::MoveTab(index, to, at, position) => {
                let before: Vec<WindowId> = self.core.windows().iter().map(|w| w.id).collect();
                let effects = self.core.move_tab(window, index, to, at);
                if let Some([x, y]) = position
                    && let Some(new) = self
                        .core
                        .windows()
                        .iter()
                        .map(|w| w.id)
                        .find(|id| !before.contains(id))
                    && let Some(w) = self.core.window_mut(new)
                {
                    w.geometry.x = x as i32;
                    w.geometry.y = y as i32;
                }
                effects
            }
            Action::Drop(target, paths, moving) => {
                self.core.drop_paths_moving(window, target, paths, moving)
            }
            Action::Home(action) => match action {
                ui_home::HomeAction::Run(id) => self.core.execute(window, id),
                ui_home::HomeAction::Browse(side) => {
                    self.core.home_browse(window, side);
                    Vec::new()
                }
                ui_home::HomeAction::Select(key) => {
                    self.core.home_select(window, &key);
                    Vec::new()
                }
                ui_home::HomeAction::Toggle(key) => {
                    if let Some(ViewModel::Home(home)) =
                        self.core.active_tab_mut(window).and_then(|t| t.model_mut())
                    {
                        home.toggle(&key);
                    }
                    Vec::new()
                }
                ui_home::HomeAction::Panel(a) => self.core.panel_action(window, a),
                ui_home::HomeAction::Rename(commit) => {
                    if commit {
                        self.core.commit_rename(window);
                    } else {
                        self.core.cancel_rename(window);
                    }
                    Vec::new()
                }
                ui_home::HomeAction::Move(reference, folder) => {
                    self.core.tree_move(window, reference, &folder);
                    Vec::new()
                }
            },
        };
        self.apply(ctx, effects);
    }

    /// Forgets the UI state of views that are gone.
    fn prune(&mut self) {
        let open: Vec<ViewId> = self
            .core
            .windows()
            .iter()
            .flat_map(|w| w.tabs.iter().map(|t| t.id))
            .collect();
        self.texts.retain(|v, _| open.contains(v));
        self.tables.retain(|v, _| open.contains(v));
        self.hexes.retain(|v, _| open.contains(v));
        self.pictures.retain(|v, _| open.contains(v));
        self.folders.retain(|v, _| open.contains(v));
        let windows: Vec<WindowId> = self.core.windows().iter().map(|w| w.id).collect();
        self.placements.retain(|w, _| windows.contains(w));
        self.builders.retain(|w, _| windows.contains(w));
    }

    /// Reads the platform's facts of the window drawn in `ui` (geometry, focus, the close button, the OS theme).
    fn read_viewport(&mut self, ui: &Ui, window: WindowId) -> bool {
        let (inner, outer, maximized, focused, close) = ui.ctx().input(|i| {
            let v = i.viewport();
            (
                v.inner_rect,
                v.outer_rect,
                v.maximized,
                v.focused,
                v.close_requested(),
            )
        });
        if let Some(inner) = inner {
            let mut g: WindowState = self
                .core
                .window(window)
                .map(|w| w.geometry.clone())
                .unwrap_or_default();
            g.width = inner.width().round().clamp(200.0, 32767.0) as i32;
            g.height = inner.height().round().clamp(200.0, 32767.0) as i32;
            g.maximized = maximized.unwrap_or(false);
            match outer {
                Some(o) => {
                    g.x = o.min.x.round().clamp(-1.0, 32767.0) as i32;
                    g.y = o.min.y.round().clamp(-1.0, 32767.0) as i32;
                }
                None => {
                    g.x = -1;
                    g.y = -1;
                }
            }
            self.core.set_geometry(window, g, outer.is_some());
        }
        let p = self.placements.entry(window).or_default();
        p.outer = outer.map(rect_of);
        p.inner_min = inner.map(|r| [r.min.x, r.min.y]);
        if focused == Some(true) {
            self.core.focus(window);
        }
        close
    }

    /// Draws one window into `ui` (the root viewport's or an immediate viewport's).
    fn draw_window(&mut self, ui: &mut Ui, window: WindowId) {
        let ctx = ui.ctx().clone();
        let close = self.read_viewport(ui, window);
        if close && !self.closing {
            let effects = self.core.close_window(window);
            if !effects.contains(&Effect::Exit) {
                // The program goes on (a prompt asks first, or other windows stay): the platform's close is
                // cancelled. For the root viewport this keeps eframe running after the root swap; the window's
                // model is gone or still asking either way.
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
            self.apply(&ctx, effects);
        }
        if self.core.window(window).is_none() {
            return;
        }
        let pal = self.palette;
        let mut actions: Vec<Action> = Vec::new();
        let mut status_hint: Option<String> = None;

        // Keys: a dialog takes Esc, Enter and F1; else the window root dispatches (D-111).
        let dialog_open = self
            .core
            .window(window)
            .is_some_and(|w| !w.dialogs.is_empty());
        let wants_keys = self
            .core
            .window(window)
            .is_some_and(|w| w.dialogs.wants_keys());
        let popup_open = egui::Popup::is_any_open(&ctx);
        let help_open = self.core.window(window).is_some_and(|w| w.help.is_some());
        let mut dialog_keys = Vec::new();
        let mut run = Vec::new();
        // A text field with the keyboard keeps the clipboard keys and Ctrl+A (D-252); the panes' Edit entries are off.
        let text_focus = ctx.text_edit_focused();
        self.core.set_text_focus(window, text_focus);
        {
            let core = &self.core;
            // The focused area (D-111): a Home path field takes the path-field shortcuts first; the folder tree
            // and the text panes take theirs while they draw.
            let area = self.focus_area(&ctx, window);
            let paste_seen = self.paste_seen.entry(window).or_default();
            ctx.input_mut(|i| {
                let shift = i.modifiers.shift;
                i.events.retain(|event| {
                    // egui turns Ctrl+C, Ctrl+X and Ctrl+V into clipboard events: without a text field they are the
                    // catalogue's keys (Copy, Paste of a folder pane; the GUI reads its own clipboard, D-252).
                    let clipboard_key = clipboard_key(event, shift, paste_seen);
                    let converted;
                    let event = match clipboard_key {
                        Some(_) if text_focus || wants_keys => return true,
                        Some((key, modifiers)) => {
                            converted = egui::Event::Key {
                                key,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers,
                            };
                            &converted
                        }
                        None => event,
                    };
                    let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    else {
                        return true;
                    };
                    let candidates = keys::shortcuts_of(*key, *modifiers);
                    let Some(shortcut) = candidates.first() else {
                        return true;
                    };
                    if wants_keys {
                        dialog_keys.push(shortcut.clone());
                        return false;
                    }
                    if dialog_open {
                        let plain = !shortcut.ctrl && !shortcut.alt && !shortcut.shift;
                        // An open combo or colour picker takes Esc first: egui closes it; so does the help.
                        if plain && shortcut.key == "Esc" && (popup_open || help_open) {
                            return true;
                        }
                        if plain && matches!(shortcut.key.as_str(), "Esc" | "Enter" | "F1") {
                            dialog_keys.push(shortcut.clone());
                            return false;
                        }
                        return true;
                    }
                    match core.dispatch_keys(window, &candidates, area) {
                        Some(id) => {
                            run.push(Invocation::new(id));
                            false
                        }
                        // D-111 rule 3: a workspace shortcut applies when no command took the key.
                        None => match core.dispatch_workspace(window, &candidates, area) {
                            Some(inv) => {
                                run.push(inv);
                                false
                            }
                            None => true,
                        },
                    }
                });
            });
        }
        for key in dialog_keys {
            let effects = self.core.dialog_key(window, &key);
            self.apply(&ctx, effects);
        }
        for inv in run {
            actions.push(Action::Run(inv));
        }

        // OS drops (UI-008): paths only; bytes-only drops are ignored.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        // Shift at the drop shows the move first (§1.9.25 W9c note 4; review F8).
        let (now, moved, shift) = ctx.input(|i| {
            (
                i.time,
                i.raw.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }
                    )
                }),
                i.modifiers.shift,
            )
        });
        let fresh = self.os_drags.entry(window).or_default().frame(
            now,
            moved,
            hovering_files,
            !dropped.is_empty(),
        );
        // Only a position the pointer reached after the drag began places it (review F4).
        let pointer = ctx
            .input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()))
            .filter(|_| fresh);
        let mut folder_view = false;

        let Some(w) = self.core.window(window) else {
            return;
        };
        let scope = w.active_tab().map_or(Scope::Home, |t| t.scope());
        let state = self.core.ctx(window);
        let show_toolbar = scope != Scope::Home && w.active_tab().is_some_and(|t| t.toolbar);
        let hide_bar = !self.core.tab_bar_visible(window);
        let help = w.help;
        let active_view = w.active_tab().map(|t| t.id);
        let message = w.active_tab().and_then(|t| t.message.clone());

        // Title (the root viewport's; the others carry theirs in their builder).
        if window == self.core.root() {
            let title = w.title();
            if title != self.root_title {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Title(title.clone()));
                self.root_title = title;
            }
        }

        // Menu bar.
        let menus = {
            let checked = |id: CommandId| self.core.checked(window, id);
            let names = self.core.workspace_names();
            let columns = self.core.column_menu(window);
            let apps = self.core.open_with_names();
            let mc = MenuContext {
                tables: self.core.tables(),
                scope,
                ctx: state,
                checked: &checked,
                workspaces: &names,
                columns: &columns,
                open_with: &apps,
                formats: &[],
            };
            menu_bar(&mc)
        };
        let mut invocations = Vec::new();
        egui::Panel::top("hk-menu")
            .frame(
                Frame::NONE
                    .fill(pal.chrome)
                    .inner_margin(Margin::symmetric(6, 2)),
            )
            .show(ui, |ui| {
                menus::bar(ui, &menus, &mut invocations, &mut status_hint)
            });

        // Tab bar.
        if !hide_bar {
            let bar = egui::Panel::top("hk-tabs")
                .frame(
                    Frame::NONE
                        .fill(pal.chrome)
                        .inner_margin(Margin::symmetric(6, 3)),
                )
                .show(ui, |ui| {
                    self.tab_bar(ui, window, &mut actions, &mut invocations)
                });
            let p = self.placements.entry(window).or_default();
            p.bar = p
                .inner_min
                .map(|m| rect_of(bar.response.rect.translate(egui::vec2(m[0], m[1]))));
        } else if let Some(p) = self.placements.get_mut(&window) {
            p.bar = None;
        }

        // Toolbar.
        if show_toolbar {
            let tables = self.core.shared_tables();
            let fresh = self
                .toolbar_cache
                .as_ref()
                .is_some_and(|(t, s, _)| Arc::ptr_eq(t, &tables) && *s == scope);
            if !fresh {
                let items = toolbar_items(&tables, scope);
                self.toolbar_cache = Some((tables, scope, items));
            }
            let items = self
                .toolbar_cache
                .as_ref()
                .map(|(_, _, items)| items.clone())
                .unwrap_or_default();
            let sessions = self.core.sessions_of(scope);
            let open_sessions = self
                .core
                .window_mut(window)
                .is_some_and(|w| std::mem::take(&mut w.focus_sessions));
            // The frame's context, computed once (G15 select: not once per button).
            let enabled = |id: CommandId| self.core.enabled_in(&state, id);
            let checked = |id: CommandId| self.core.checked(window, id);
            let t = ToolbarCtx {
                tables: self.core.tables(),
                scope,
                enabled: &enabled,
                checked: &checked,
                timing: self.core.hints(),
                pal: &pal,
                sessions: &sessions,
                open_sessions,
            };
            let mut hover = self.hover;
            let out = egui::Panel::top("hk-toolbar")
                .frame(
                    Frame::NONE
                        .fill(pal.bg)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| {
                    menus::toolbar(ui, &t, &items, &mut hover, &mut status_hint)
                })
                .inner;
            self.hover = hover;
            for a in out {
                match a {
                    ToolbarAction::Run(inv) => invocations.push(inv),
                    ToolbarAction::Menu(item) => actions.push(Action::ToolbarMenu(item)),
                }
            }
        }

        // The Filters edit of a folder view's toolbar (M2 W9c, D-250).
        if show_toolbar
            && matches!(scope, Scope::Folder | Scope::Sync)
            && let Some(view) = active_view
        {
            let (presets, recent) = self.core.filters_choices();
            let focus = self
                .core
                .window_mut(window)
                .is_some_and(|w| std::mem::take(&mut w.focus_filters));
            if let Some(ViewModel::Folder(m)) = self.core.tab_mut(view).and_then(|t| t.model_mut())
            {
                let edit = m.filters_edit();
                egui::Panel::top(format!("hk-filters-{}", view.0))
                    .frame(
                        Frame::NONE
                            .fill(pal.chrome)
                            .inner_margin(Margin::symmetric(8, 2)),
                    )
                    .show(ui, |ui| {
                        if let Some(a) =
                            filters_field(ui, view, edit, &presets, &recent, focus, &pal)
                        {
                            actions.push(a);
                        }
                    });
            }
        }

        // Status bar, message panel, log panel.
        let items = self.core.status(window);
        let jobs = self.core.jobs(window);
        egui::Panel::bottom("hk-status")
            .frame(
                Frame::NONE
                    .fill(pal.chrome)
                    .inner_margin(Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| {
                for a in status::bar(ui, &items, status_hint.as_deref(), &jobs, &pal) {
                    actions.push(Action::Status(a));
                }
            });
        if let (Some(m), Some(view)) = (&message, active_view) {
            egui::Panel::bottom("hk-message")
                .frame(Frame::NONE)
                .show(ui, |ui| {
                    if let Some(a) = status::message(ui, m, &pal) {
                        actions.push(Action::Message(view, a));
                    }
                });
        }
        // The Find in Files results panel of the window (D-308).
        if let Some(w) = self.core.window_mut(window)
            && w.find_files.shown
        {
            let height = if w.find_files.height > 0.0 {
                w.find_files.height
            } else {
                180.0
            };
            let replace = self.files_replace.entry(window).or_default();
            let panel = egui::Panel::bottom(format!("hk-find-files-{}", window.0))
                .resizable(true)
                .default_size(height)
                .min_size(80.0)
                .frame(
                    Frame::NONE
                        .fill(pal.t_bg)
                        .inner_margin(Margin::symmetric(6, 3)),
                )
                .show(ui, |ui| {
                    find_files::show(ui, &mut w.find_files, replace, &pal)
                });
            let h = panel.response.rect.height();
            if (h - height).abs() > 0.5 {
                actions.push(Action::FindFiles(find_files::FilesAction::Height(h)));
            }
            for a in panel.inner {
                actions.push(Action::FindFiles(a));
            }
        }
        if let Some(view) = active_view
            && let Some(ViewModel::Folder(m)) = self.core.tab(view).and_then(|t| t.model())
            && m.show_log()
        {
            let height = m.log_height() as f32;
            let panel = egui::Panel::bottom(format!("hk-log-{}", view.0))
                .resizable(true)
                .default_size(height)
                .min_size(60.0)
                .frame(Frame::NONE)
                .show(ui, |ui| log::show(ui, self.core.log(), &pal));
            if let Some(a) = panel.inner {
                actions.push(Action::Log(view, a));
            }
            let h = f64::from(panel.response.rect.height());
            if (h - f64::from(height)).abs() > 0.5 {
                actions.push(Action::LogHeight(view, h));
            }
        }

        // Help panel (with a dialog open: over the dialog, below, or the modal would take its clicks).
        if let Some(topic) = help.filter(|_| !dialog_open) {
            egui::Panel::right("hk-help")
                .resizable(true)
                .default_size(380.0)
                .min_size(240.0)
                .frame(
                    Frame::NONE
                        .fill(pal.chrome)
                        .inner_margin(Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| match help::show(ui, topic, &pal) {
                    Some(help::HelpAction::Open(id)) => actions.push(Action::Help(id)),
                    Some(help::HelpAction::Close) => actions.push(Action::CloseHelp),
                    None => {}
                });
        }

        // The header of a Folder Sync view (FS-001): the preset, the plan's state, Run Sync.
        if let Some(view) = active_view
            && let Some(ViewModel::Folder(m)) = self.core.tab(view).and_then(|t| t.model())
            && m.is_sync()
        {
            let run_enabled = !dialog_open && state_run_enabled(&state);
            let panel = egui::Panel::top(format!("hk-sync-{}", view.0))
                .frame(
                    Frame::NONE
                        .fill(pal.chrome)
                        .inner_margin(Margin::symmetric(10, 4)),
                )
                .show(ui, |ui| sync_view::header(ui, m, run_enabled, &pal));
            match panel.inner {
                Some(sync_view::SyncHeaderAction::Run) => {
                    invocations.push(Invocation::new(CommandId::Run));
                }
                Some(sync_view::SyncHeaderAction::Preset(preset)) => {
                    self.core.sync_preset(view, preset);
                }
                None => {}
            }
        }

        // The Find Filename panel of a folder view (D-241): above the tree, not modal.
        if let Some(view) = active_view
            && !dialog_open
            && let Some(ViewModel::Folder(m)) = self.core.tab_mut(view).and_then(|t| t.model_mut())
            && m.show_find()
        {
            let panel = egui::Panel::top(format!("hk-find-{}", view.0))
                .frame(
                    Frame::NONE
                        .fill(pal.chrome)
                        .inner_margin(Margin::symmetric(10, 4)),
                )
                .show(ui, |ui| find_panel::show(ui, m.find_panel(), &pal));
            match panel.inner {
                Some(find_panel::FindAction::Next) => {
                    invocations.push(Invocation::new(CommandId::FindFilenameNext));
                }
                Some(find_panel::FindAction::Prev) => {
                    invocations.push(Invocation::new(CommandId::FindFilenamePrev));
                }
                Some(find_panel::FindAction::Close) => m.close_find(),
                Some(find_panel::FindAction::Help) => {
                    actions.push(Action::Help(crate::find_panel::FIND_TOPIC.to_owned()));
                }
                None => {}
            }
        }

        // The Legend panel of a folder view (D-242): beside the tree, not modal.
        if let Some(view) = active_view
            && let Some(ViewModel::Folder(m)) = self.core.tab(view).and_then(|t| t.model())
            && m.show_legend()
        {
            let sections = if m.is_sync() {
                crate::legend::sync_sections()
            } else {
                crate::legend::sections()
            };
            let panel = egui::Panel::right(format!("hk-legend-{}", view.0))
                .resizable(true)
                .default_size(300.0)
                .min_size(200.0)
                .frame(
                    Frame::NONE
                        .fill(pal.chrome)
                        .inner_margin(Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| legend::show(ui, &sections, &pal));
            match panel.inner {
                Some(legend::LegendAction::Close) => {
                    invocations.push(Invocation::new(CommandId::Legend));
                }
                Some(legend::LegendAction::Help) => {
                    actions.push(Action::Help(crate::legend::LEGEND_TOPIC.to_owned()));
                }
                None => {}
            }
        }

        // The view.
        let background = if scope == Scope::Home {
            pal.chrome
        } else {
            pal.bg
        };
        let mut drop_fields: Option<[egui::Rect; 2]> = None;
        // Where paths from the OS land in a folder view under the pointer (M2 W9c, D-251).
        let mut folder_target: Option<DropTarget> = None;
        // The side a drop on a picture view replaces (§1.13.8; W13b review finding 17).
        let mut picture_target: Option<DropTarget> = None;
        let mut drop_types: Vec<(hikasa_session::sessions::SessionType, egui::Rect)> = Vec::new();
        let tree_font = {
            let f = &self.core.theme().fonts.folder;
            let size = if f.use_system { 13.0 } else { f.size as f32 };
            egui::FontId::new(size, self.slot_fonts.family(FontSlot::Folder))
        };
        let editor_family = self.slot_fonts.family(FontSlot::Editor);
        // The hex panes draw with the theme's hex slot (D-160; W13a left them on the editor's).
        let hex_family = self.slot_fonts.family(FontSlot::Hex);
        // M2 W13b (D-363): a picture tab not shown drops its tiles (keeps its overviews); the shown one may hold the
        // window's budget minus what the others keep. The shown view is always given room for its visible set: the
        // hidden views' overviews go first, least recently shown first (W13b review finding 1).
        let service = self.core.service_handle();
        let mut picture_others = 0u64;
        self.picture_clock += 1;
        let clock = self.picture_clock;
        if let Some(w) = self.core.window_mut(window) {
            let active = w.active;
            let mut need = 0u64;
            let mut hidden: Vec<(u64, usize)> = Vec::new();
            for (k, tab) in w.tabs.iter_mut().enumerate() {
                let id = tab.id;
                let Some(ViewModel::Picture(m)) = tab.model_mut() else {
                    continue;
                };
                if k == active {
                    need = m.demand();
                    self.pictures.entry(id).or_default().last_shown = clock;
                    continue;
                }
                if let Some(st) = self.pictures.get_mut(&id) {
                    m.hidden(&mut picture::Sink::new(&ctx, st));
                    picture_others += m.tiles.bytes();
                    hidden.push((st.last_shown, k));
                }
            }
            hidden.sort_unstable();
            for (_, k) in hidden {
                if picture_others + need <= crate::picture::tiles::TEXTURE_BUDGET {
                    break;
                }
                let tab = &mut w.tabs[k];
                let id = tab.id;
                if let Some(ViewModel::Picture(m)) = tab.model_mut()
                    && let Some(st) = self.pictures.get_mut(&id)
                {
                    picture_others -= m.tiles.bytes();
                    m.evict_all(&mut picture::Sink::new(&ctx, st));
                }
            }
        }
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(background))
            .show(ui, |ui| {
                let Some(view) = active_view else {
                    return;
                };
                let dispatch_ctx = state;
                let tables = self.core.shared_tables();
                let apps = self.core.open_with_names();
                let Some(tab) = self.core.tab_mut(view) else {
                    return;
                };
                let keys = !dialog_open;
                if tab.pending_tab().is_some() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Loading…").color(pal.muted));
                    });
                    return;
                }
                match tab.model_mut() {
                    Some(ViewModel::Home(home)) => {
                        let menu = |h: &crate::home::HomeModel| {
                            let mut c = dispatch_ctx;
                            h.ctx_into(&mut c);
                            let none = |_: CommandId| None;
                            crate::menus::tree_menu(&MenuContext {
                                tables: &tables,
                                scope: Scope::Home,
                                ctx: c,
                                checked: &none,
                                workspaces: &[],
                                columns: &[],
                                open_with: &[],
                                formats: &[],
                            })
                        };
                        let out = home::show(ui, view, home, &pal, &menu);
                        if let [Some(l), Some(r)] = out.fields {
                            drop_fields = Some([l, r]);
                        }
                        drop_types = out.types;
                        invocations.extend(out.invocations);
                        actions.extend(out.actions.into_iter().map(Action::Home));
                    }
                    Some(ViewModel::Folder(m)) => {
                        let s = m.status();
                        let columns = m.column_menu();
                        let none = |_: CommandId| None;
                        let view_scope = m.scope();
                        let header_menu = crate::menus::header_menu(&MenuContext {
                            tables: &tables,
                            scope: view_scope,
                            ctx: dispatch_ctx,
                            checked: &none,
                            workspaces: &[],
                            columns: &columns,
                            open_with: &[],
                            formats: &[],
                        });
                        // The pane context menus (D-253), built only while one is open.
                        let pane_menu = |area: crate::commands::Area, c: &crate::commands::Ctx| {
                            crate::menus::pane_menu(
                                &MenuContext {
                                    tables: &tables,
                                    scope: view_scope,
                                    ctx: *c,
                                    checked: &none,
                                    workspaces: &[],
                                    columns: &columns,
                                    open_with: &apps,
                                    formats: &[],
                                },
                                area,
                            )
                        };
                        let panes = folder::PaneMenus {
                            ctx: dispatch_ctx,
                            menu: &pane_menu,
                        };
                        let st = self.folders.entry(view).or_default();
                        st.os_pointer = pointer;
                        folder_view = true;
                        let dispatch = |key: &hikasa_session::settings::Shortcut| {
                            tables.dispatch(view_scope, &dispatch_ctx, key, FocusArea::ViewMain)
                        };
                        invocations.extend(folder::show(
                            ui,
                            view,
                            m,
                            st,
                            &s,
                            &dispatch,
                            &pal,
                            &tree_font,
                            keys,
                            &header_menu,
                            &panes,
                        ));
                        if std::mem::take(&mut st.commit_rename) {
                            actions.push(Action::CommitRename);
                        }
                        if let Some((drag, target, shift)) = st.row_drop.take() {
                            actions.push(Action::RowDrop(drag, target, shift));
                        }
                        folder_target = st.os_target;
                    }
                    Some(ViewModel::Text(m)) => {
                        let s = m.status();
                        let view_scope = m.scope();
                        let st = self.texts.entry(view).or_default();
                        let dispatch = |key: &hikasa_session::settings::Shortcut| {
                            tables.dispatch(view_scope, &dispatch_ctx, key, FocusArea::ViewMain)
                        };
                        // The text areas' context menus (D-312), built only while one is open.
                        let none = |_: CommandId| None;
                        let formats = m.format_names().to_vec();
                        let menu = |area: crate::commands::Area, c: &crate::commands::Ctx| {
                            crate::menus::pane_menu(
                                &MenuContext {
                                    tables: &tables,
                                    scope: view_scope,
                                    ctx: *c,
                                    checked: &none,
                                    workspaces: &[],
                                    columns: &[],
                                    open_with: &apps,
                                    formats: &formats,
                                },
                                area,
                            )
                        };
                        let menus = text::TextMenus {
                            ctx: dispatch_ctx,
                            menu: &menu,
                        };
                        invocations.extend(text::show(
                            ui,
                            view,
                            m,
                            st,
                            &s,
                            &pal,
                            &editor_family,
                            &dispatch,
                            keys,
                            &menus,
                        ));
                    }
                    // M2 W13b: the picture panes; the textures of the window's other picture views count against the
                    // budget (D-363).
                    Some(ViewModel::Picture(m)) => {
                        let st = self.pictures.entry(view).or_default();
                        // The context menu's checks and radios are the view's (W13b review finding 17).
                        let menu = |area: crate::commands::Area,
                                    c: &crate::commands::Ctx,
                                    checked: &dyn Fn(CommandId) -> Option<bool>| {
                            crate::menus::pane_menu(
                                &MenuContext {
                                    tables: &tables,
                                    scope: Scope::Picture,
                                    ctx: *c,
                                    checked,
                                    workspaces: &[],
                                    columns: &[],
                                    open_with: &apps,
                                    formats: &[],
                                },
                                area,
                            )
                        };
                        let budget =
                            crate::picture::tiles::TEXTURE_BUDGET.saturating_sub(picture_others);
                        invocations.extend(picture::show(
                            ui,
                            view,
                            m,
                            st,
                            &service,
                            &pal,
                            budget,
                            keys,
                            &menu,
                            dispatch_ctx,
                        ));
                        picture_target = Some(DropTarget::PictureSide(
                            m.drop_side(pointer.map(|p| (f64::from(p.x), f64::from(p.y)))),
                        ));
                    }
                    Some(ViewModel::Table(m)) => {
                        let st = self.tables.entry(view).or_default();
                        let dispatch = |key: &hikasa_session::settings::Shortcut| {
                            tables.dispatch(Scope::Table, &dispatch_ctx, key, FocusArea::ViewMain)
                        };
                        for id in
                            table::show(ui, view, m, st, &pal, &editor_family, &dispatch, keys)
                        {
                            invocations.push(Invocation::new(id));
                        }
                    }
                    Some(ViewModel::Hex(m)) => {
                        let st = self.hexes.entry(view).or_default();
                        let dispatch = |key: &hikasa_session::settings::Shortcut| {
                            tables.dispatch(Scope::Hex, &dispatch_ctx, key, FocusArea::ViewMain)
                        };
                        let formats = m.format_names().to_vec();
                        let none = |_: CommandId| None;
                        let menu = |area: crate::commands::Area, c: &crate::commands::Ctx| {
                            crate::menus::pane_menu(
                                &MenuContext {
                                    tables: &tables,
                                    scope: Scope::Hex,
                                    ctx: *c,
                                    checked: &none,
                                    workspaces: &[],
                                    columns: &[],
                                    open_with: &apps,
                                    formats: &formats,
                                },
                                area,
                            )
                        };
                        let menus = hex::HexMenus {
                            ctx: dispatch_ctx,
                            menu: &menu,
                        };
                        invocations.extend(hex::show(
                            ui,
                            view,
                            m,
                            st,
                            &pal,
                            &hex_family,
                            &dispatch,
                            keys,
                            &menus,
                        ));
                        // A drop on a pane or its header replaces that side (UI-008).
                        folder_target = pointer.and_then(|p| {
                            st.drop_rects
                                .iter()
                                .find(|(_, r)| r.contains(p))
                                .map(|(side, _)| DropTarget::PanePath(*side))
                        });
                    }
                    None => {}
                }
            });

        // Drops: a Home path field under the pointer, else the window.
        if hovering_files && let Some(p) = pointer {
            ui.painter().rect_stroke(
                ctx.content_rect().shrink(2.0),
                4.0,
                egui::Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
            let _ = p;
        }
        // With a dialog open a drop belongs to the dialog's fields, not to the view under it.
        if !dropped.is_empty() && !dialog_open {
            let target = match (drop_fields, pointer) {
                (Some([l, _]), Some(p)) if l.contains(p) => {
                    DropTarget::PathField(FieldId::HomeLeft)
                }
                (Some([_, r]), Some(p)) if r.contains(p) => {
                    DropTarget::PathField(FieldId::HomeRight)
                }
                _ => match folder_target.or(picture_target) {
                    Some(target) => target,
                    // A folder view with no fresh position asks for the pane (review F4).
                    None if folder_view && pointer.is_none() => DropTarget::PaneAsk,
                    None => pointer
                        .and_then(|p| drop_types.iter().find(|(_, r)| r.contains(p)))
                        .map_or(DropTarget::Window, |(kind, _)| DropTarget::HomeType(*kind)),
                },
            };
            actions.push(Action::Drop(target, dropped, shift));
        }

        // Notices.
        let now = ctx.input(|i| i.time);
        if let Some(w) = self.core.window_mut(window) {
            let visible: Vec<crate::notify::Notice> =
                w.notices.visible(now).into_iter().cloned().collect();
            let refs: Vec<&crate::notify::Notice> = visible.iter().collect();
            if let Some((key, action)) = notify::show(ui, &refs, &pal) {
                actions.push(Action::Notice(key, action));
            }
            if let Some(at) = w.notices.next_expiry(now) {
                ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.05)));
            }
        }

        // The top dialog.
        if let Some(w) = self.core.window_mut(window) {
            let depth = w.dialogs.len();
            if let Some(top) = w.dialogs.top_mut() {
                for input in dialog::show(ui, &mut **top, depth, &pal) {
                    actions.push(Action::Dialog(input));
                }
            }
        }
        if let Some(topic) = help.filter(|_| dialog_open) {
            match help::show_over_dialog(ui, topic, &pal) {
                Some(help::HelpAction::Open(id)) => actions.push(Action::Help(id)),
                Some(help::HelpAction::Close) => actions.push(Action::CloseHelp),
                None => {}
            }
        }

        for inv in invocations {
            actions.push(Action::Run(inv));
        }
        for action in actions {
            self.act(&ctx, window, action);
        }
        // Effects the views raised while drawing (a selection that moved, a gap bar that opened).
        let effects = self.core.dirty_effects();
        if !effects.is_empty() {
            self.apply(&ctx, effects);
        }
    }

    /// The tab bar: select, close (×, middle click), context menu, drag (reorder, move, D-107), New Tab.
    fn tab_bar(
        &mut self,
        ui: &mut Ui,
        window: WindowId,
        actions: &mut Vec<Action>,
        run: &mut Vec<Invocation>,
    ) {
        let Some(w) = self.core.window(window) else {
            return;
        };
        let captions: Vec<String> = w.tabs.iter().map(|t| t.caption()).collect();
        let active = w.active;
        let others = self.core.other_windows(window);
        let mut rects = Vec::new();
        let mut released: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, caption) in captions.iter().enumerate() {
                let caption = crate::shared::shown_name(caption);
                let short: String = if caption.chars().count() > TAB_CAPTION {
                    let mut s: String = caption.chars().take(TAB_CAPTION - 1).collect();
                    s.push('…');
                    s
                } else {
                    caption.clone()
                };
                let r = ui
                    .add(Button::selectable(i == active, short).sense(Sense::click_and_drag()))
                    .hint(&caption);
                rects.push(r.rect);
                if r.clicked() {
                    actions.push(Action::Activate(i));
                }
                if r.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                }
                if r.drag_started() {
                    self.drag = Some((window, i));
                }
                if r.drag_stopped() && self.drag == Some((window, i)) {
                    released = Some(i);
                }
                r.context_menu(|ui| {
                    let state = self.core.ctx(window);
                    let checked = |id: CommandId| self.core.checked(window, id);
                    let mc = MenuContext {
                        tables: self.core.tables(),
                        scope: self.core.scope(window),
                        ctx: crate::commands::Ctx {
                            tabs: state.tabs,
                            ..state
                        },
                        checked: &checked,
                        workspaces: &[],
                        columns: &[],
                        open_with: &[],
                        formats: &[],
                    };
                    let mut hint = None;
                    menus::items(ui, &tab_menu(&mc, i, &others), run, &mut hint);
                });
                if ui
                    .add(Button::new(RichText::new("×").small()).frame(false))
                    .hint("Close Tab")
                    .clicked()
                {
                    actions.push(Action::CloseTab(i));
                }
                ui.add_space(6.0);
            }
            let plus = ui.add(Button::new("+").frame(false));
            let tip = crate::commands::hint_text(
                &self.core.tables().caption(Scope::Global, CommandId::TabNew),
                self.core
                    .tables()
                    .shortcut(Scope::Global, CommandId::TabNew),
                "",
                true,
            );
            if plus.hint(tip).clicked() {
                run.push(Invocation::new(CommandId::TabNew));
            }
        });
        let Some(index) = released else {
            return;
        };
        self.drag = None;
        let Some(local) = ui
            .ctx()
            .input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
        else {
            return;
        };
        let bar = ui.min_rect();
        if bar.contains(local) {
            // Reorder inside the bar.
            // The insertion point among the tabs before the move (`move_tab` shifts it past the moved tab).
            let at = rects
                .iter()
                .position(|r| local.x < r.center().x)
                .unwrap_or(rects.len());
            if at != index && at != index + 1 {
                actions.push(Action::MoveTab(index, Some(window), Some(at), None));
            }
            return;
        }
        let global = self
            .placements
            .get(&window)
            .and_then(|p| p.inner_min)
            .map(|m| [m[0] + local.x, m[1] + local.y]);
        let all: Vec<(WindowId, Option<ScreenRect>, Option<ScreenRect>)> = self
            .core
            .windows()
            .iter()
            .map(|w| {
                let p = self.placements.get(&w.id);
                (w.id, p.and_then(|p| p.outer), p.and_then(|p| p.bar))
            })
            .collect();
        match resolve_tab_drop(window, &all, global) {
            TabDrop::Nothing => {}
            TabDrop::Move(to) => actions.push(Action::MoveTab(index, Some(to), None, None)),
            TabDrop::NewWindow(at) => actions.push(Action::MoveTab(index, None, None, Some(at))),
        }
    }

    /// The viewport builder of a window that is not the root: geometry from the model when its viewport opens
    /// (or after the model moved it), then only its title follows the model.
    fn builder(&mut self, window: WindowId) -> ViewportBuilder {
        let Some(w) = self.core.window(window) else {
            return ViewportBuilder::default();
        };
        let title = w.title();
        let b = self.builders.entry(window).or_insert_with(|| {
            let g = &w.geometry;
            let mut b = ViewportBuilder::default()
                .with_app_id("hikasa")
                .with_inner_size([g.width as f32, g.height as f32])
                .with_min_inner_size([640.0, 400.0]);
            if g.x >= 0 && g.y >= 0 {
                b = b.with_position([g.x as f32, g.y as f32]);
            }
            b
        });
        b.clone().with_title(title)
    }

    /// The focus area of a window's active view (D-111): a Home path field, the folder tree or the text panes
    /// when one of them has the keyboard focus, else the window root.
    fn focus_area(&self, ctx: &egui::Context, window: WindowId) -> FocusArea {
        let Some(focused) = ctx.memory(|m| m.focused()) else {
            return FocusArea::Root;
        };
        let Some(tab) = self.core.active_tab(window) else {
            return FocusArea::Root;
        };
        match tab.model() {
            Some(ViewModel::Home(_))
                if focused == home::field_id(tab.id, false)
                    || focused == home::field_id(tab.id, true) =>
            {
                FocusArea::PathField
            }
            Some(ViewModel::Home(_)) if focused == home::tree_id(tab.id) => FocusArea::SessionsTree,
            Some(ViewModel::Home(_)) if focused == home::search_id(tab.id) => FocusArea::HomeSearch,
            Some(ViewModel::Text(_)) if focused == text::pane_id(tab.id) => FocusArea::ViewMain,
            Some(ViewModel::Table(_)) if focused == table::grid_id(tab.id) => FocusArea::ViewMain,
            Some(ViewModel::Hex(_)) if focused == hex::panes_id(tab.id) => FocusArea::ViewMain,
            Some(ViewModel::Folder(_)) if focused == folder::tree_id(tab.id) => FocusArea::ViewMain,
            // A path edit, the Filters edit, the find panel: a text field keeps its editing keys (D-252).
            _ if ctx.text_edit_focused() => FocusArea::TextField,
            _ => FocusArea::Root,
        }
    }

    /// The work of a frame before drawing (eframe's `logic`): the OS theme, the mailbox, the jobs, the bench.
    fn step(&mut self, ctx: &egui::Context) {
        if let Some(fonts) = self.next_fonts.take() {
            self.slot_fonts = fonts;
        }
        self.core.set_now(ctx.input(|i| i.time));
        self.core.set_timer_now(self.clock.elapsed().as_secs_f64());
        hint::set_timing(ctx, self.core.hints());
        // The OS theme (egui's system theme), re-evaluated when it changes (D-113).
        let dark = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        if dark != self.os_dark {
            self.os_dark = dark;
            let effects = self.core.set_os_dark(dark.unwrap_or(false));
            self.apply(ctx, effects);
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
        // The auto-save timer needs a frame also in an idle window; eframe calls `logic` for a repaint while the
        // window is hidden (no `ui`), so this reaches a minimized window too.
        if let Some(wait) = self.core.autosave_wake_in() {
            ctx.request_repaint_after(Duration::from_secs_f64(wait.max(0.05)));
        }
        if self.core.settling() {
            // A job woke the window before its result was published (microseconds): look again at once.
            ctx.request_repaint_after(SETTLE_POLL);
        } else if self.core.busy() {
            ctx.request_repaint_after(BUSY_POLL);
        }
        if self
            .bench
            .as_mut()
            .is_some_and(BenchState::take_second_window)
        {
            let root = self.core.root();
            let effects = self.core.execute(root, CommandId::WindowNew);
            // The comparison stays the root window's active tab; the new window holds Home.
            self.core.focus(root);
            self.apply(ctx, effects);
        }
        if let Some(b) = self.bench.as_mut() {
            b.drive_options(&mut self.core);
            b.drive_w9a(&mut self.core);
            b.drive_w9b(&mut self.core);
            b.drive(
                &self.core,
                &mut self.texts,
                &mut self.tables,
                &mut self.folders,
            );
            b.drive_edit(ctx, &mut self.core, &mut self.texts);
            b.drive_hex(&mut self.core);
            b.drive_picture(&mut self.core);
            b.drive_mixed(&mut self.core);
            match b.repaint_after() {
                Some(wait) => ctx.request_repaint_after(wait),
                None => ctx.request_repaint(),
            }
        }
    }

    /// Draws every window (eframe's `ui`): the root window in `ui`, the others in their immediate viewports.
    fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let root = self.core.root();
        self.draw_window(ui, root);
        // The other windows, each in its immediate viewport (D-106).
        let others: Vec<WindowId> = self.core.windows().iter().skip(1).map(|w| w.id).collect();
        for window in others {
            if self.core.window(window).is_none() || self.closing {
                continue;
            }
            let builder = self.builder(window);
            ctx.show_viewport_immediate(viewport_of(window), builder, |ui, _class| {
                self.draw_window(ui, window);
            });
        }
        self.prune();
        // The startup workspace loads once the window is up (§1.4.12).
        self.core.after_first_frame();
        if self.deferred.pending() {
            // The first frame is laid out: the first-run work runs now, on its own thread (never awaited).
            let _detached = self.deferred.start();
        }
        if let Some(b) = self.bench.as_mut() {
            b.set_window(serde_json::json!({
                "size_points": [ctx.content_rect().width(), ctx.content_rect().height()],
                "pixels_per_point": ctx.pixels_per_point(),
            }));
            b.end_frame(&self.texts, &self.tables);
            b.end_frame_hex(&self.hexes);
            b.end_frame_picture(&mut self.core);
            b.end_frame_mixed();
            b.end_frame_edit(&self.core, &self.texts);
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
}

/// Applies the palette's visuals to both of egui's styles and fixes egui's theme to the scheme's base (D-113):
/// egui never switches styles by itself when the OS theme changes; under `follow_os` the core picks the scheme
/// and this runs again.
fn set_theme(ctx: &egui::Context, palette: &Palette, scheme: &hikasa_session::settings::Scheme) {
    let visuals = palette.visuals(scheme);
    ctx.set_theme(if visuals.dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.all_styles_mut(|s| s.visuals = visuals.clone());
}

impl eframe::App for HikasaApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(b) = self.bench.as_mut() {
            b.begin_frame(frame.info().cpu_usage);
        }
        self.step(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn on_exit(&mut self) {
        // The window went away without a close request we saw (e.g. the session ended): views still report.
        if !self.closing {
            self.closing = true;
            let _ = self.core.shutdown();
        }
        // The layout write of the shutdown, and the log lines not saved yet.
        self.core.flush(Duration::from_secs(2));
        if let Some(handle) = self.watch.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hex_pointer;
#[cfg(test)]
mod tests_picture_review_ui;
#[cfg(test)]
mod tests_pointer;
#[cfg(test)]
mod tests_table;
#[cfg(test)]
mod tests_table_perf;

/// `Run`'s predicate in the window's context.
fn state_run_enabled(ctx: &crate::commands::Ctx) -> bool {
    crate::commands::Predicate::PlanReady.holds(ctx)
}
