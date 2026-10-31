//! Draws the menus, context menus and toolbars that `menus.rs` assembles (D-110), with hints (UI-007).

use egui::{AtomExt as _, Button, ComboBox, Response, Sense, Ui, vec2};

use crate::commands::{CommandId, CommandTables, HintTiming, Scope, hint_text, lookup};
use crate::menus::{Check, Menu, MenuItem, ToolItem, ToolbarMenuItem, display_filter};
use crate::ui::hint::Hint as _;
use crate::ui::icons;
use crate::ui::theme::Palette;
use crate::window::Invocation;

/// The hover of the hint timing (UI-007): the widget hovered and since when (frame time).
#[derive(Debug, Clone, Copy, Default)]
pub struct Hover {
    id: Option<egui::Id>,
    since: f64,
}

impl Hover {
    /// Shows `text` as the hint of `response` per `timing` (the window repaints until it shows or hides).
    pub fn hint(
        &mut self,
        response: Response,
        text: impl Into<egui::WidgetText>,
        timing: &HintTiming,
    ) -> Response {
        self.hint_with(response, timing, |r| r.on_hover_text(text))
    }

    /// Like [`Hover::hint`], `show` drawing the hint when it is due.
    pub fn hint_with(
        &mut self,
        response: Response,
        timing: &HintTiming,
        show: impl FnOnce(Response) -> Response,
    ) -> Response {
        let now = response.ctx.input(|i| i.time);
        if !response.hovered() {
            if self.id == Some(response.id) {
                self.id = None;
            }
            return response;
        }
        if self.id != Some(response.id) {
            self.id = Some(response.id);
            self.since = now;
        }
        let hovered = now - self.since;
        if timing.visible(hovered) {
            if timing.hide_ms > 0 {
                let left = f64::from(timing.show_ms + timing.hide_ms) / 1000.0 - hovered;
                response
                    .ctx
                    .request_repaint_after(std::time::Duration::from_secs_f64(left.max(0.01)));
            }
            show(response)
        } else {
            if timing.enabled && hovered * 1000.0 < f64::from(timing.show_ms) {
                let left = f64::from(timing.show_ms) / 1000.0 - hovered;
                response
                    .ctx
                    .request_repaint_after(std::time::Duration::from_secs_f64(left.max(0.01)));
            }
            response
        }
    }
}

/// Draws menu items; pushes the invocation of the clicked one; `status_hint` gets the hovered item's hint.
pub fn items(
    ui: &mut Ui,
    list: &[MenuItem],
    run: &mut Vec<Invocation>,
    status_hint: &mut Option<String>,
) {
    for item in list {
        match item {
            MenuItem::Separator => {
                ui.separator();
            }
            MenuItem::Submenu {
                caption,
                items: sub,
            } => {
                ui.menu_button(caption, |ui| {
                    ui.set_min_width(260.0);
                    items(ui, sub, run, status_hint);
                });
            }
            MenuItem::Command {
                invocation,
                caption,
                shortcut,
                check,
                enabled,
                description,
            } => {
                let mark = match check {
                    Check::None => None,
                    Check::Toggle(on) => Some(if *on { "✔" } else { "" }),
                    Check::Radio(on) => Some(if *on { "●" } else { "" }),
                };
                let mut button = match mark {
                    Some(m) => Button::new((m.atom_size(vec2(16.0, 0.0)), caption.as_str())),
                    None => Button::new(caption.as_str()),
                };
                if let Some(s) = shortcut {
                    button = button.shortcut_text(s.as_str());
                }
                let response = ui.add_enabled(*enabled, button);
                if response.hovered() && !description.is_empty() {
                    *status_hint = Some(description.clone());
                }
                if response.clicked() {
                    run.push(invocation.clone());
                    ui.close();
                }
            }
        }
    }
}

/// The menu bar.
pub fn bar(
    ui: &mut Ui,
    menus: &[Menu],
    run: &mut Vec<Invocation>,
    status_hint: &mut Option<String>,
) {
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in menus {
            ui.menu_button(menu.title.caption(), |ui| {
                ui.set_min_width(300.0);
                items(ui, &menu.items, run, status_hint);
            });
        }
    });
}

/// What the toolbar asked for.
pub enum ToolbarAction {
    /// Run a command.
    Run(Invocation),
    /// An entry of the toolbar's context menu.
    Menu(ToolbarMenuItem),
}

/// Everything a toolbar needs.
pub struct ToolbarCtx<'a> {
    /// The named sessions of the view's type (`SessionsCombo`).
    pub sessions: &'a [String],
    /// `SessionsCombo` asked to open the drop-down this frame.
    pub open_sessions: bool,
    /// The command tables.
    pub tables: &'a CommandTables,
    /// The view type.
    pub scope: Scope,
    /// Availability of a command.
    pub enabled: &'a dyn Fn(CommandId) -> bool,
    /// Check state of a command.
    pub checked: &'a dyn Fn(CommandId) -> Option<bool>,
    /// Hint timing.
    pub timing: HintTiming,
    /// The palette.
    pub pal: &'a Palette,
}

impl ToolbarCtx<'_> {
    fn hint(&self, id: CommandId) -> String {
        hint_text(
            &self.tables.caption(self.scope, id),
            self.tables.shortcut(self.scope, id),
            self.tables.description(self.scope, id),
            self.timing.show_shortcut,
        )
    }
}

/// One icon button (with its label under it with large buttons).
fn icon_button(
    ui: &mut Ui,
    t: &ToolbarCtx<'_>,
    id: CommandId,
    large: bool,
    hover: &mut Hover,
    status_hint: &mut Option<String>,
) -> bool {
    let row = lookup(t.scope, id);
    let icon = row.and_then(|r| r.icon);
    let enabled = (t.enabled)(id);
    let checked = (t.checked)(id);
    let label = row
        .and_then(|r| r.toolbar_label)
        .map_or_else(|| t.tables.caption(t.scope, id), str::to_owned);
    let size = if large {
        vec2(64.0, 44.0)
    } else {
        vec2(28.0, 26.0)
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = if enabled {
        response
    } else {
        response.on_disabled_hover_text("")
    };
    let pal = t.pal;
    let colour = if !enabled {
        pal.muted.gamma_multiply(0.6)
    } else if checked == Some(true) {
        pal.accent
    } else {
        pal.fg
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        if checked == Some(true) {
            painter.rect_filled(rect.shrink(1.0), 4.0, pal.accent.gamma_multiply(0.15));
        } else if enabled && response.hovered() {
            painter.rect_filled(rect.shrink(1.0), 4.0, pal.hover);
        }
        let icon_side = 18.0;
        let icon_rect = icons::square(
            egui::pos2(rect.center().x - icon_side / 2.0, rect.top() + 4.0),
            icon_side,
        );
        match icon {
            Some(icon) => icons::paint(&painter, icon_rect, icon, colour),
            None => {
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label.chars().take(2).collect::<String>(),
                    egui::FontId::proportional(11.0),
                    colour,
                );
            }
        }
        if large {
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 9.0),
                egui::Align2::CENTER_CENTER,
                &label,
                egui::FontId::proportional(10.5),
                colour,
            );
        }
    }
    let hint = t.hint(id);
    if response.hovered() {
        *status_hint = Some(t.tables.description(t.scope, id).to_owned());
    }
    let response = hover.hint(response, &hint, &t.timing);
    enabled && response.clicked()
}

/// Draws a toolbar; returns what was asked for.
pub fn toolbar(
    ui: &mut Ui,
    t: &ToolbarCtx<'_>,
    items: &[ToolItem],
    hover: &mut Hover,
    status_hint: &mut Option<String>,
) -> Vec<ToolbarAction> {
    let mut out = Vec::new();
    let large = t.tables.view(t.scope).custom.large_buttons;
    let response = ui
        .horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for item in items {
                match item {
                    ToolItem::Command(id) => {
                        if icon_button(ui, t, *id, large, hover, status_hint) {
                            out.push(ToolbarAction::Run(Invocation::new(*id)));
                        }
                    }
                    ToolItem::DisplayFilter => {
                        ui.separator();
                        let control = display_filter(t.tables, t.scope);
                        for id in &control.buttons {
                            if icon_button(ui, t, *id, large, hover, status_hint) {
                                out.push(ToolbarAction::Run(Invocation::new(*id)));
                            }
                        }
                        if !control.presets.is_empty()
                            && control.mode
                                != hikasa_session::settings::commands::DisplayFilterMode::Toggles
                        {
                            let current = control
                                .presets
                                .iter()
                                .chain(&control.buttons)
                                .find(|id| (t.checked)(**id) == Some(true))
                                .map_or_else(
                                    || "Custom filter".to_owned(),
                                    |id| t.tables.caption(t.scope, *id),
                                );
                            ComboBox::from_id_salt("hk-filter")
                                .selected_text(current)
                                .width(210.0)
                                .show_ui(ui, |ui| {
                                    for id in &control.presets {
                                        let on = (t.checked)(*id) == Some(true);
                                        if ui
                                            .selectable_label(on, t.tables.caption(t.scope, *id))
                                            .hint(t.tables.description(t.scope, *id))
                                            .clicked()
                                        {
                                            out.push(ToolbarAction::Run(Invocation::new(*id)));
                                        }
                                    }
                                })
                                .response
                                .hint("Display filter");
                        }
                        for id in &control.toggles {
                            if icon_button(ui, t, *id, large, hover, status_hint) {
                                out.push(ToolbarAction::Run(Invocation::new(*id)));
                            }
                        }
                    }
                    ToolItem::Sessions => {
                        ui.separator();
                        let combo = egui::ComboBox::from_id_salt("hk-sessions")
                            .selected_text("Sessions")
                            .width(120.0);
                        let response = combo.show_ui(ui, |ui| {
                            for name in t.sessions {
                                if ui.selectable_label(false, name).clicked()
                                    && let Ok(reference) =
                                        hikasa_session::sessions::SessionRef::parse(name)
                                {
                                    out.push(ToolbarAction::Run(Invocation::with(
                                        CommandId::SessionsCombo,
                                        crate::window::CommandArg::Session(reference),
                                    )));
                                }
                            }
                        });
                        if t.open_sessions {
                            response.response.request_focus();
                        }
                    }
                    ToolItem::FolderMode => {
                        let modes: Vec<CommandId> = crate::commands::rows_for(t.scope)
                            .filter(|r| r.group == crate::commands::Group::FolderMode)
                            .map(|r| r.id)
                            .collect();
                        if modes.is_empty() {
                            continue;
                        }
                        ui.separator();
                        let current = modes
                            .iter()
                            .find(|id| (t.checked)(**id) == Some(true))
                            .map_or_else(String::new, |id| t.tables.caption(t.scope, *id));
                        ComboBox::from_id_salt("hk-folder-mode")
                            .selected_text(current)
                            .width(240.0)
                            .show_ui(ui, |ui| {
                                for id in &modes {
                                    let on = (t.checked)(*id) == Some(true);
                                    if ui
                                        .selectable_label(on, t.tables.caption(t.scope, *id))
                                        .hint(t.tables.description(t.scope, *id))
                                        .clicked()
                                    {
                                        out.push(ToolbarAction::Run(Invocation::new(*id)));
                                    }
                                }
                            })
                            .response
                            .hint("Folders");
                    }
                }
            }
        })
        .response;
    // UI-004: the toolbar's context menu.
    let menu = crate::menus::toolbar_menu(t.tables, t.scope);
    let response = ui.interact(
        response.rect,
        ui.scope_id().with("hk-toolbar-menu"),
        Sense::click(),
    );
    response.context_menu(|ui| {
        for entry in &menu {
            let (mark, on) = match entry {
                ToolbarMenuItem::Mode(_, on) => (Some("●"), *on),
                ToolbarMenuItem::LargeButtons(on) => (Some("✔"), *on),
                _ => (None, false),
            };
            let button = match mark {
                Some(m) => Button::new((
                    (if on { m } else { "" }).atom_size(vec2(16.0, 0.0)),
                    entry.caption(),
                )),
                None => Button::new(entry.caption()),
            };
            if ui.add(button).clicked() {
                out.push(ToolbarAction::Menu(*entry));
                ui.close();
            }
        }
    });
    out
}
