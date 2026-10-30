//! The Home view (M2 W4b, D-124): type buttons, search, the sessions tree and the management panel. The fields are
//! passed verbatim to the model; everything that reads or writes a file is a job of the app core.

use egui::{
    Align, Button, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, TextEdit, Ui,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{SessionRef, SessionType};

use crate::app::PanelAction;
use crate::commands::CommandId;
use crate::home::{HomeModel, PanelMode, PanelTarget, TreeNode, TreeRow};
use crate::menus::{MenuItem, OPENABLE_TYPES};
use crate::ui::hint::Hint as _;
use crate::ui::icons::{self, IconId};
use crate::ui::theme::Palette;
use crate::window::Invocation;

/// What the Home view asked for.
pub enum HomeAction {
    /// A command.
    Run(CommandId),
    /// A row was selected.
    Select(String),
    /// A row was opened or closed.
    Toggle(String),
    /// A panel button.
    Panel(PanelAction),
    /// The in-place rename ended.
    Rename(bool),
    /// A session was dragged onto a folder.
    Move(SessionRef, String),
}

/// What [`show`] returns.
#[derive(Default)]
pub struct HomeOut {
    /// The actions.
    pub actions: Vec<HomeAction>,
    /// The commands of context menus.
    pub invocations: Vec<Invocation>,
    /// The left and right path fields (drop targets, UI-008).
    pub fields: [Option<egui::Rect>; 2],
    /// The type buttons (drop targets).
    pub types: Vec<(SessionType, egui::Rect)>,
}

/// The id of a Home path field (its keyboard focus: the path-field shortcuts, D-111).
pub fn field_id(view: ViewId, right: bool) -> egui::Id {
    egui::Id::unique(("hikasa-home-field", view.0, right))
}

/// The id of the search box.
pub fn search_id(view: ViewId) -> egui::Id {
    egui::Id::unique(("hikasa-home-search", view.0))
}

/// The id of the tree (its keyboard focus: Enter, Del, F2).
pub fn tree_id(view: ViewId) -> egui::Id {
    egui::Id::unique(("hikasa-home-tree", view.0))
}

fn icon_of(row: &TreeRow) -> Option<IconId> {
    match &row.node {
        TreeNode::Heading(_) | TreeNode::AutoGroup(_) => None,
        TreeNode::NewType { kind, .. } => Some(IconId::of_session_type(*kind)),
        TreeNode::Folder(_) => Some(IconId::Folder),
        TreeNode::Named { kind, .. } | TreeNode::AutoSaved { kind, .. } => {
            Some(IconId::of_session_type(*kind))
        }
    }
}

/// Draws the Home view. `menu` gives the tree's context menu for the current selection.
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    home: &mut HomeModel,
    pal: &Palette,
    menu: &dyn Fn(&HomeModel) -> Vec<MenuItem>,
) -> HomeOut {
    let mut out = HomeOut::default();
    let panel_mode = home.panel_mode;
    let width = ui.available_width();
    let tree_width = match panel_mode {
        PanelMode::Hidden => width,
        PanelMode::Shown => (width * 0.42).clamp(240.0, 460.0),
        PanelMode::Maximized => 0.0,
    };
    ui.horizontal_top(|ui| {
        if panel_mode != PanelMode::Maximized {
            ui.allocate_ui_with_layout(
                egui::vec2(tree_width, ui.available_height()),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.set_width(tree_width);
                    Frame::NONE
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| tree_side(ui, view, home, pal, menu, &mut out));
                },
            );
        }
        if panel_mode != PanelMode::Hidden {
            ui.separator();
            ui.vertical(|ui| {
                Frame::NONE
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| panel(ui, view, home, pal, &mut out));
            });
        }
    });
    out
}

fn tree_side(
    ui: &mut Ui,
    view: ViewId,
    home: &mut HomeModel,
    pal: &Palette,
    menu: &dyn Fn(&HomeModel) -> Vec<MenuItem>,
    out: &mut HomeOut,
) {
    ui.horizontal(|ui| {
        for (kind, caption) in OPENABLE_TYPES {
            let response = ui
                .add(Button::new(caption))
                .hint("Click for an empty session of this type; drop paths here to fill it.");
            out.types.push((kind, response.rect));
            if response.clicked() {
                out.actions.push(HomeAction::Select(format!("new:{kind}")));
            }
        }
    });
    ui.add_space(4.0);
    let search = ui.add(
        TextEdit::singleline(&mut home.search)
            .id(search_id(view))
            .hint_text("Search sessions")
            .desired_width(f32::INFINITY),
    );
    if home.focus_search {
        home.focus_search = false;
        search.request_focus();
    }
    ui.add_space(4.0);
    let keys = ui.ctx().memory(|m| m.focused()) == Some(tree_id(view));
    if keys {
        // Arrows move the selection: the tree holds the keyboard focus.
        let delta = ui.ctx().input(|i| {
            i32::from(i.key_pressed(egui::Key::ArrowDown))
                - i32::from(i.key_pressed(egui::Key::ArrowUp))
        });
        if delta != 0 {
            let visible = home.visible();
            let at = home
                .selection()
                .and_then(|s| visible.iter().position(|r| r.key == s.key));
            let next = at.map_or(0, |i| {
                (i as i64 + i64::from(delta)).clamp(0, visible.len() as i64 - 1) as usize
            });
            if let Some(row) = visible.get(next) {
                out.actions.push(HomeAction::Select(row.key.clone()));
            }
        }
    }
    let rows: Vec<TreeRow> = home.visible().into_iter().cloned().collect();
    let selected = home.selection().map(|r| r.key.clone());
    let row_height = 22.0;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            for row in &rows[range] {
                tree_row(
                    ui,
                    view,
                    home,
                    row,
                    selected.as_deref() == Some(row.key.as_str()),
                    pal,
                    menu,
                    out,
                );
            }
        });
    if ui.ctx().input(|i| i.pointer.any_released()) {
        home.dragging = None;
    }
}

#[allow(clippy::too_many_arguments)] // one row of the tree: the frame's state
fn tree_row(
    ui: &mut Ui,
    view: ViewId,
    home: &mut HomeModel,
    row: &TreeRow,
    selected: bool,
    pal: &Palette,
    menu: &dyn Fn(&HomeModel) -> Vec<MenuItem>,
    out: &mut HomeOut,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 22.0),
        Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::Default);
    if selected {
        ui.painter().rect_filled(rect, 3.0, pal.hover);
    }
    let x = rect.left() + 4.0 + 14.0 * row.depth as f32;
    let mut text_x = x;
    if row.expandable() {
        let open = home.expanded.contains(&row.key);
        let id = if open {
            IconId::Collapse
        } else {
            IconId::Expand
        };
        let square =
            egui::Rect::from_min_size(egui::pos2(x, rect.top() + 4.0), egui::vec2(14.0, 14.0));
        icons::paint(ui.painter(), square, id, pal.muted);
        text_x += 16.0;
    }
    if let Some(id) = icon_of(row) {
        let square =
            egui::Rect::from_min_size(egui::pos2(text_x, rect.top() + 4.0), egui::vec2(14.0, 14.0));
        icons::paint(ui.painter(), square, id, pal.fg);
        text_x += 18.0;
    }
    let editing = home.rename.as_ref().is_some_and(|e| e.key == row.key);
    if editing {
        let field = egui::Rect::from_min_max(
            egui::pos2(text_x, rect.top() + 1.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
        );
        if let Some(edit) = &mut home.rename {
            let r = ui.put(field, TextEdit::singleline(&mut edit.text));
            r.request_focus();
            let (enter, esc) = ui.ctx().input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if esc {
                out.actions.push(HomeAction::Rename(false));
            } else if enter {
                out.actions.push(HomeAction::Rename(true));
            }
        }
        return;
    }
    let colour = if matches!(row.node, TreeNode::Heading(_) | TreeNode::AutoGroup(_)) {
        pal.muted
    } else {
        pal.fg
    };
    let mut caption = crate::shared::shown_name(&row.caption());
    if let TreeNode::Named {
        favorite, locked, ..
    } = &row.node
    {
        if *favorite {
            caption = format!("★ {caption}");
        }
        if *locked {
            caption.push_str(" (locked)");
        }
    }
    if matches!(
        row.node,
        TreeNode::NewType {
            customized: true,
            ..
        }
    ) {
        caption.push_str(" (customized)");
    }
    ui.painter().text(
        egui::pos2(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        caption,
        FontId::proportional(13.0),
        colour,
    );
    if response.clicked() || response.secondary_clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(tree_id(view)));
        if row.expandable() && response.clicked() {
            out.actions.push(HomeAction::Toggle(row.key.clone()));
        }
        out.actions.push(HomeAction::Select(row.key.clone()));
    }
    if response.double_clicked() && !row.expandable() {
        out.actions.push(HomeAction::Run(CommandId::TreeOpen));
    }
    if let TreeNode::Named { reference, .. } = &row.node
        && response.drag_started()
    {
        home.dragging = Some(reference.clone());
    }
    if let TreeNode::Folder(path) = &row.node
        && ui.ctx().input(|i| i.pointer.any_released())
        && ui
            .ctx()
            .input(|i| i.pointer.latest_pos())
            .is_some_and(|p| rect.contains(p))
        && let Some(reference) = home.dragging.take()
    {
        out.actions.push(HomeAction::Move(reference, path.clone()));
    }
    response.context_menu(|ui| {
        let mut hint = None;
        crate::ui::menus::items(ui, &menu(home), &mut out.invocations, &mut hint);
    });
}

fn panel(ui: &mut Ui, view: ViewId, home: &mut HomeModel, pal: &Palette, out: &mut HomeOut) {
    let width = ui.available_width().min(640.0);
    ui.set_width(width);
    let target = home.panel.target.clone();
    let heading = match &target {
        PanelTarget::None => "Compare two folders or two files".to_owned(),
        PanelTarget::New(kind) => format!("New {}", crate::home::type_caption(*kind)),
        PanelTarget::Named(r) => r.to_string(),
        PanelTarget::AutoSaved(_) => "Auto-saved session".to_owned(),
    };
    ui.label(RichText::new(heading).size(18.0).strong().color(pal.fg));
    ui.add_space(4.0);
    if target == PanelTarget::None {
        ui.label(
            RichText::new("Paths may be relative to the folder Hikasa was started in.")
                .color(pal.muted),
        );
    }
    if home.panel.loading {
        ui.label(RichText::new("Loading…").color(pal.muted));
    }
    ui.add_space(8.0);
    let read_only = matches!(target, PanelTarget::AutoSaved(_));
    let ready = home.ready() && !home.resolving();
    let mut enter = false;
    if matches!(target, PanelTarget::Named(_)) {
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(72.0, 24.0),
                egui::Label::new(RichText::new("Description").color(pal.muted)),
            );
            ui.add(TextEdit::singleline(&mut home.panel.description).desired_width(f32::INFINITY));
        });
        ui.add_space(4.0);
    }
    for (caption, right) in [("Left", false), ("Right", true)] {
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(72.0, 24.0),
                egui::Label::new(RichText::new(caption).color(pal.muted)),
            );
            let value = if right {
                &mut home.panel.right
            } else {
                &mut home.panel.left
            };
            let hint = if right {
                "/path/to/right"
            } else {
                "/path/to/left"
            };
            let field = ui.add_enabled(
                !read_only,
                TextEdit::singleline(value)
                    .id(field_id(view, right))
                    .hint_text(hint)
                    .font(FontId::monospace(13.0))
                    .desired_width(f32::INFINITY),
            );
            out.fields[usize::from(right)] = Some(field.rect);
            if field.changed() {
                home.error = None;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                enter = true;
            }
        });
        ui.add_space(4.0);
    }
    ui.add_space(6.0);
    let mut press = |action: PanelAction| out.actions.push(HomeAction::Panel(action));
    ui.horizontal_wrapped(|ui| {
        let open_label = if target == PanelTarget::None {
            "Compare"
        } else {
            "Open"
        };
        let compare = ui
            .add_enabled(
                ready,
                Button::new(RichText::new(open_label).strong()).min_size(egui::vec2(96.0, 28.0)),
            )
            .hint("Opens the two paths; the comparison replaces the Home view.");
        if compare.clicked() || (enter && ready) {
            press(PanelAction::Open);
        }
        match &target {
            PanelTarget::Named(_) => {
                if ui.button("Save").clicked() {
                    press(PanelAction::Save);
                }
                if ui.button("Save As…").clicked() {
                    press(PanelAction::SaveAs);
                }
                if ui.button("Factory Defaults").clicked() {
                    press(PanelAction::FactoryDefaults);
                }
                if ui
                    .add_enabled(home.panel.changed(), Button::new("Cancel"))
                    .clicked()
                {
                    press(PanelAction::Cancel);
                }
                let locked = home.panel.session.as_ref().is_some_and(|s| s.locked);
                let mut on = locked;
                if ui.checkbox(&mut on, "Locked").changed() {
                    press(PanelAction::SetLocked(on));
                }
            }
            PanelTarget::AutoSaved(_) => {
                if ui.button("Open as New").clicked() {
                    press(PanelAction::OpenAsNew);
                }
                if ui.button("Delete").clicked() {
                    press(PanelAction::Delete);
                }
            }
            _ => {}
        }
    });
    if home.resolving() {
        ui.label(RichText::new("Opening…").color(pal.muted));
    }
    if let Some(error) = &home.error {
        ui.add_space(6.0);
        Frame::NONE
            .fill(pal.error_bg)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(error).color(pal.error));
            });
    }
}
