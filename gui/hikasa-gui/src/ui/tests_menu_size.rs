//! Every menu has an adequate size (0.2.0, maintainer request). Each view type's menu bar with every submenu, every
//! context menu of the view's areas (pane, gutter, header, thumbnail, value panel), the tab menu, the sessions tree's
//! menu and the toolbar's menu, enumerated from the menu assembly (`menus.rs`), not by hand, are opened by pointer
//! through the real drawing code (`ui::menus::bar_panel`, `ui::menus::items` in egui's context-menu popup, the real
//! toolbar) at the window's style and fonts, on a 1280 × 720 screen (points) at 1, 1.5 and 2 pixels per point, and
//! measured from what egui laid out (the popup's area, the widgets' rects, the text galleys and the arrow shapes):
//!
//! - a label is drawn in full: its galley is the caption, not elided, one line;
//! - a shortcut keeps [`min_gap`] from its label, a mark (check, radio) and a submenu arrow do not overlap it;
//! - every item's row lies inside the menu frame (inside its margin) and is at least the style's interact height;
//! - the menu is at least as wide as its widest row needs, and at least [`MIN_MENU_WIDTH`];
//! - the menu, and a submenu, lie on the screen; a menu taller than the screen scrolls (and only then), its rows out
//!   of view are measured after a scroll, and its scroll bar keeps clear of the texts.
//!
//! The picture panes' menu, which `ui/picture.rs` draws in an area of its own, is opened in the whole window
//! ([`the_picture_view_menu_has_an_adequate_size`]). The text headers' encoding, line-ending and format pickers, the
//! status bar's menus and the table grid's column menu are written in their views' drawing code, not assembled from
//! the catalogue, and are not covered here.
//!
//! `HIKASA_MENU_SHOTS=<dir>` also writes a PNG of each opened menu (with its parents), rasterised from egui's own
//! meshes and font atlas, as `<dir>/<scale>x/<view>/<menu path>.png`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use egui::epaint::{ClippedPrimitive, ClippedShape, Primitive};
use egui::{
    Color32, ColorImage, Event, Id, LayerId, Order, PointerButton, Pos2, RawInput, Rect, Sense,
    Shape, TextureId, Vec2, ViewportId, pos2, vec2,
};
use hikasa_core::Side;

use super::HikasaApp;
use super::tests::{new_app, pointer_button};
use crate::commands::{Area, CommandId, CommandTables, Ctx, Group, Scope, lookup};
use crate::menus::{
    Check, Menu, MenuContext, MenuItem, ToolbarMenuItem, header_menu, menu_bar, pane_menu,
    render_text, tab_menu, toolbar_items, toolbar_menu, tree_menu,
};
use crate::tests::World;
use crate::window::WindowId;

/// The screen, in points (the smallest a window is expected to be maximised on).
const SCREEN: Vec2 = vec2(1280.0, 720.0);
/// The scales (pixels per point) measured.
const SCALES: [f32; 3] = [1.0, 1.5, 2.0];
/// No menu is narrower.
const MIN_MENU_WIDTH: f32 = 120.0;
/// Rounding slack of a comparison, in points.
const EPS: f32 = 0.5;
/// Where a context menu is opened.
const CONTEXT_AT: Pos2 = pos2(60.0, 80.0);

/// The least gap between a label and its shortcut: twice the item spacing.
fn min_gap(style: &egui::Style) -> f32 {
    2.0 * style.spacing.item_spacing.x
}

// ---- the menus -----------------------------------------------------------------------------------------------------

/// How a menu is opened.
#[derive(Clone)]
enum Opener {
    /// The `.1`-th menu of the bar `.0`: a click on its title.
    Bar(Vec<Menu>, usize),
    /// A context menu with these items: a right click on a widget.
    Context,
    /// The toolbar's context menu: a right click on the real toolbar.
    Toolbar,
    /// The whole window: a right click at this point.
    Window(Pos2),
}

/// One menu to open and measure.
#[derive(Clone)]
struct Subject {
    scope: Scope,
    /// The menu's name in a message ("Text Compare ▸ Edit", "Hex Compare ▸ HexPane(Left) menu").
    name: String,
    opener: Opener,
    items: Vec<MenuItem>,
}

/// Every context-menu area with each side it is drawn for. A new area must be added here and to [`drawn_in`].
fn all_areas() -> Vec<Area> {
    let mut all = vec![
        Area::Tab,
        Area::Toolbar,
        Area::SessionTree,
        Area::FolderHeader,
        Area::FolderCentre,
        Area::TextGutter,
        Area::HexThumbnail,
        Area::HexValues,
        Area::SyncPane(Side::Center),
    ];
    for side in [Side::Left, Side::Right] {
        all.extend([
            Area::FolderPane(side),
            Area::SyncPane(side),
            Area::TextPane(side),
            Area::TextHeader(side),
            Area::HexPane(side),
            Area::FileHeader(side),
        ]);
    }
    all.extend(
        hikasa_picture::Pane::ALL
            .iter()
            .map(|p| Area::PicturePane(*p)),
    );
    all
}

/// The view types whose drawing code opens the context menu of `area` (exhaustive: a new area fails to compile here).
fn drawn_in(area: Area) -> &'static [Scope] {
    match area {
        Area::Tab | Area::Toolbar => &Scope::VIEWS,
        Area::SessionTree => &[Scope::Home],
        Area::FolderHeader => &[Scope::Folder, Scope::Sync],
        Area::FolderPane(_) | Area::FolderCentre => &[Scope::Folder],
        Area::SyncPane(_) => &[Scope::Sync],
        Area::TextPane(_) | Area::TextGutter | Area::TextHeader(_) => {
            &[Scope::Text, Scope::TextEdit]
        }
        Area::HexPane(_) | Area::HexThumbnail | Area::HexValues | Area::FileHeader(_) => {
            &[Scope::Hex]
        }
        Area::PicturePane(_) => &[Scope::Picture],
    }
}

/// The menus of a view type: the bar's, the context menus of its areas (each distinct list once), the toolbar's.
fn subjects(world: &World, tables: &CommandTables, scope: Scope) -> Vec<Subject> {
    // Every checkable entry checked: its mark is drawn and measured.
    let checked = |id: CommandId| {
        lookup(scope, id)
            .filter(|r| r.group != Group::None)
            .map(|_| true)
    };
    let columns: Vec<_> = crate::folder::columns::ColumnId::ALL
        .into_iter()
        .filter(|c| *c != crate::folder::columns::ColumnId::Name)
        .enumerate()
        .map(|(k, c)| (c, k % 2 == 0))
        .collect();
    let kind = match scope {
        Scope::Text | Scope::TextEdit => "text",
        Scope::Hex => "hex",
        _ => "",
    };
    let formats: Vec<String> = world
        .service
        .format_list()
        .formats
        .into_iter()
        .filter(|f| f.enabled && f.kind == kind)
        .map(|f| f.name)
        .collect();
    let workspaces = [
        "Daily Review".to_owned(),
        "Release 0.2 Checklist".to_owned(),
    ];
    let open_with = [
        "Visual Studio Code".to_owned(),
        "GNU Image Manipulation Program".to_owned(),
    ];
    let ctx = Ctx {
        scope,
        tabs: 2,
        windows: 2,
        ready: true,
        tree_named: true,
        tree_auto: true,
        tree_folder: true,
        tree_new: true,
        ..Ctx::default()
    };
    let mc = MenuContext {
        tables,
        scope,
        ctx,
        checked: &checked,
        workspaces: &workspaces,
        columns: &columns,
        open_with: &open_with,
        formats: &formats,
    };
    let title = scope.type_title();
    let mut out = Vec::new();
    let bar = menu_bar(&mc);
    for (k, menu) in bar.iter().enumerate() {
        out.push(Subject {
            scope,
            name: format!("{title} ▸ {}", menu.title.caption()),
            opener: Opener::Bar(bar.clone(), k),
            items: menu.items.clone(),
        });
    }
    // A window caption as long as the tab menu's Move to Window ▸ shows one (60 characters, the last an ellipsis).
    let others = [(
        WindowId(2),
        format!(
            "{}…",
            "nightly-build-output-2026-11-18 ↔ nightly-build-outp"
        ),
    )];
    let mut seen = BTreeSet::new();
    for area in all_areas() {
        if !drawn_in(area).contains(&scope) {
            continue;
        }
        let items = match area {
            Area::Toolbar => {
                if toolbar_items(tables, scope).is_empty() {
                    continue;
                }
                out.push(Subject {
                    scope,
                    name: format!("{title} ▸ toolbar menu"),
                    opener: Opener::Toolbar,
                    items: toolbar_menu(tables, scope)
                        .into_iter()
                        .map(toolbar_item)
                        .collect(),
                });
                continue;
            }
            Area::Tab => tab_menu(&mc, 0, &others),
            Area::SessionTree => tree_menu(&mc),
            Area::FolderHeader => header_menu(&mc),
            area => pane_menu(&mc, area),
        };
        if items.is_empty()
            || !seen.insert(render_text(&[Menu {
                title: crate::menus::MenuTitle::Edit,
                items: items.clone(),
            }]))
        {
            continue;
        }
        out.push(Subject {
            scope,
            name: format!("{title} ▸ {area:?} menu"),
            opener: Opener::Context,
            items,
        });
    }
    out
}

/// A toolbar menu entry as the menu item it is drawn like (a mark when it is a mode or the large-buttons check).
fn toolbar_item(entry: ToolbarMenuItem) -> MenuItem {
    let check = match entry {
        ToolbarMenuItem::Mode(_, on) => Check::Radio(on),
        ToolbarMenuItem::LargeButtons(on) => Check::Toggle(on),
        _ => Check::None,
    };
    MenuItem::Command {
        invocation: crate::window::Invocation::new(CommandId::ProgramOptions),
        caption: entry.caption().to_owned(),
        shortcut: None,
        check,
        enabled: true,
        description: String::new(),
    }
}

/// The submenus under `items`, depth first: each one's path of (row, caption), rows counted without separators.
fn submenu_paths(
    items: &[MenuItem],
    prefix: &[(usize, String)],
    out: &mut Vec<Vec<(usize, String)>>,
) {
    let mut row = 0;
    for item in items {
        match item {
            MenuItem::Separator => continue,
            MenuItem::Submenu {
                caption,
                items: sub,
            } => {
                let mut path = prefix.to_vec();
                path.push((row, caption.clone()));
                out.push(path.clone());
                submenu_paths(sub, &path, out);
            }
            MenuItem::Command { .. } => {}
        }
        row += 1;
    }
}

/// The items of the submenu at `path`.
fn items_at<'a>(items: &'a [MenuItem], path: &[(usize, String)]) -> &'a [MenuItem] {
    let Some(((row, _), rest)) = path.split_first() else {
        return items;
    };
    match items
        .iter()
        .filter(|i| !matches!(i, MenuItem::Separator))
        .nth(*row)
    {
        Some(MenuItem::Submenu { items: sub, .. }) => items_at(sub, rest),
        _ => panic!("no submenu at row {row}"),
    }
}

// ---- one frame and what it laid out ---------------------------------------------------------------------------------

/// A popup layer of a frame: its area, its shapes and its widgets.
struct Layer {
    id: LayerId,
    rect: Rect,
    shapes: Vec<ClippedShape>,
    widgets: Vec<egui::WidgetRect>,
}

impl Layer {
    /// The rects of the items' rows: the focusable clickable widgets (not the popup's own area), in drawing order.
    fn rows(&self) -> Vec<Rect> {
        self.widgets
            .iter()
            .filter(|w| w.sense.senses_click() && w.sense.is_focusable())
            .map(|w| w.rect)
            .collect()
    }
}

/// What a frame laid out.
struct Capture {
    /// The foreground (popup) layers with widgets.
    layers: Vec<Layer>,
    /// The texts of the background layer (the menu bar's titles) with their rects.
    titles: Vec<(String, Rect)>,
    /// The toolbar's rect (the toolbar opener).
    toolbar: Rect,
    output: egui::FullOutput,
}

/// One drawn text: its string, its rect, its galley and its clip rect.
struct Text {
    text: String,
    rect: Rect,
    galley: std::sync::Arc<egui::Galley>,
    clip: Rect,
}

fn texts(shapes: &[ClippedShape]) -> Vec<Text> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<Text>) {
        match shape {
            Shape::Text(t) if !t.galley.text().is_empty() => out.push(Text {
                text: t.galley.text().to_owned(),
                rect: t.galley.rect.translate(t.pos.to_vec2()),
                galley: t.galley.clone(),
                clip,
            }),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// The visible part of a menu's rows: the smallest clip of its texts (a scrolling menu's viewport), else its area.
fn viewport(layer: &Layer) -> Rect {
    texts(&layer.shapes)
        .into_iter()
        .map(|t| t.clip.intersect(layer.rect))
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or(layer.rect)
}

/// The bounds of the shapes that are neither text, a filled rect (a row's background, the frame) nor a line (a
/// separator): the submenu arrows.
fn marks(shapes: &[ClippedShape]) -> Vec<Rect> {
    fn walk(shape: &Shape, out: &mut Vec<Rect>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            Shape::Path(_) | Shape::Mesh(_) | Shape::Circle(_) | Shape::Ellipse(_) => {
                out.push(shape.visual_bounding_rect());
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, &mut out);
    }
    out
}

/// The scroll bars of a menu: its tall, narrow filled rects.
fn bars(shapes: &[ClippedShape], row_height: f32) -> Vec<Rect> {
    fn walk(shape: &Shape, row_height: f32, out: &mut Vec<Rect>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, row_height, out)),
            Shape::Rect(r) if r.rect.width() < 20.0 && r.rect.height() > 2.0 * row_height => {
                out.push(r.rect);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, row_height, &mut out);
    }
    out
}

/// A headless window of one scale drawing one subject's menus.
struct Harness {
    world: World,
    app: HikasaApp,
    ctx: egui::Context,
    ppp: f32,
    time: f64,
    /// Shots are taken.
    shots: bool,
}

impl Harness {
    fn new(ppp: f32, shots: bool) -> Harness {
        let world = World::new();
        let ctx = egui::Context::default();
        let app = new_app(&world, &ctx);
        // What `Effect::ApplyFonts` sets at startup.
        ctx.set_fonts(super::fonts::definitions(app.core.fonts()));
        Harness {
            world,
            app,
            ctx,
            ppp,
            time: 0.0,
            shots,
        }
    }

    fn raw(&mut self, events: Vec<Event>) -> RawInput {
        self.time += 0.1;
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            time: Some(self.time),
            events,
            ..RawInput::default()
        };
        raw.viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        raw
    }

    /// One frame of `subject`'s opener; captures the layers before egui hands the shapes out.
    fn frame(&mut self, subject: &Subject, events: Vec<Event>) -> Capture {
        let raw = self.raw(events);
        let tables = self.app.core.tables().clone();
        let pal = self.app.palette;
        let mut toolbar = Rect::NOTHING;
        let mut layers = Vec::new();
        let mut titles = Vec::new();
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let mut run = Vec::new();
            let mut hint = None;
            match &subject.opener {
                Opener::Window(_) => {
                    app.step(ui.ctx());
                    app.draw(ui);
                }
                Opener::Bar(menus, _) => {
                    super::menus::bar_panel(ui, &pal, menus, &mut run, &mut hint);
                }
                Opener::Context => {
                    let response = ui.interact(
                        ui.max_rect(),
                        Id::unique("hk-menu-size-target"),
                        Sense::click(),
                    );
                    response.context_menu(|ui| {
                        super::menus::items(ui, &subject.items, &mut run, &mut hint);
                    });
                }
                Opener::Toolbar => {
                    let items = toolbar_items(&tables, subject.scope);
                    let enabled = |_: CommandId| true;
                    let checked = |_: CommandId| None;
                    let t = super::menus::ToolbarCtx {
                        sessions: &[],
                        open_sessions: false,
                        tables: &tables,
                        scope: subject.scope,
                        enabled: &enabled,
                        checked: &checked,
                        timing: crate::commands::HintTiming::default(),
                        pal: &pal,
                    };
                    let mut hover = super::menus::Hover::default();
                    super::menus::toolbar(ui, &t, &items, &mut hover, &mut hint);
                    toolbar = ui.min_rect();
                }
            }
            let ctx = ui.ctx();
            let ids: Vec<LayerId> = ctx.viewport(|v| {
                v.this_pass
                    .widgets
                    .layer_ids()
                    .filter(|l| l.order == Order::Foreground)
                    .filter(|l| v.this_pass.widgets.get_layer(*l).next().is_some())
                    .collect()
            });
            for id in ids {
                let Some(rect) = ctx.memory(|m| m.area_rect(id.id)) else {
                    continue;
                };
                let shapes = ctx.graphics(|g| {
                    g.get(id)
                        .map(|l| l.all_entries().cloned().collect())
                        .unwrap_or_default()
                });
                let widgets =
                    ctx.viewport(|v| v.this_pass.widgets.get_layer(id).copied().collect());
                layers.push(Layer {
                    id,
                    rect,
                    shapes,
                    widgets,
                });
            }
            let background = ctx.graphics(|g| {
                g.get(LayerId::background())
                    .map(|l| l.all_entries().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            });
            titles = texts(&background)
                .into_iter()
                .map(|t| (t.text, t.rect))
                .collect();
        });
        output.textures_delta.clear();
        Capture {
            layers,
            titles,
            toolbar,
            output,
        }
    }

    /// Closes every popup and lets the menus' state lapse.
    fn reset(&mut self, subject: &Subject) {
        egui::Popup::close_all(&self.ctx);
        self.frame(subject, vec![Event::PointerGone]);
        self.time += 1.0;
        self.frame(subject, Vec::new());
        self.frame(subject, Vec::new());
    }

    fn settle(&mut self, subject: &Subject) -> Capture {
        self.frame(subject, Vec::new());
        self.frame(subject, Vec::new());
        self.frame(subject, Vec::new())
    }

    fn click(&mut self, subject: &Subject, at: Pos2, button: PointerButton) -> Capture {
        self.frame(subject, vec![Event::PointerMoved(at)]);
        self.frame(subject, vec![pointer_button(at, button, true)]);
        self.frame(subject, vec![pointer_button(at, button, false)]);
        self.settle(subject)
    }

    /// Scrolls the menu `layer` (whose items are `items`) until its row `row` shows, with the wheel over one of its
    /// shown commands; the frame after.
    fn scroll_to(
        &mut self,
        subject: &Subject,
        mut capture: Capture,
        layer: LayerId,
        items: &[MenuItem],
        row: usize,
    ) -> Result<Capture, String> {
        let want = expected(items);
        for _ in 0..40 {
            let menu = capture
                .layers
                .iter()
                .find(|l| l.id == layer)
                .ok_or("the menu closed while scrolling")?;
            let rows = menu.rows();
            let view = viewport(menu);
            let target = *rows.get(row).ok_or_else(|| format!("no row {row}"))?;
            if view.expand(EPS).contains_rect(target) {
                return Ok(capture);
            }
            let over = rows
                .iter()
                .zip(&want)
                .find(|(r, w)| !w.submenu && view.contains_rect(**r))
                .map(|(r, _)| r.center())
                .ok_or("no command row shows to scroll over")?;
            let delta = if target.center().y > view.center().y {
                -60.0
            } else {
                60.0
            };
            self.frame(subject, vec![Event::PointerMoved(over)]);
            self.frame(
                subject,
                vec![Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, delta),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            capture = self.settle(subject);
        }
        Err(format!("row {row} never scrolled into view"))
    }

    /// Opens `subject` and the submenus of `path`; the last frame and the open layers, the menu's first.
    fn open(
        &mut self,
        subject: &Subject,
        path: &[(usize, String)],
    ) -> Result<(Capture, Vec<LayerId>), String> {
        self.reset(subject);
        let first = self.frame(subject, Vec::new());
        let at = match &subject.opener {
            Opener::Bar(menus, k) => {
                let title = menus[*k].title.caption();
                first
                    .titles
                    .iter()
                    .find(|(t, _)| t == title)
                    .map(|(_, r)| r.center())
                    .ok_or_else(|| format!("the menu bar shows no title {title:?}"))?
            }
            Opener::Context => CONTEXT_AT,
            Opener::Window(at) => *at,
            Opener::Toolbar => first.toolbar.left_center() + vec2(12.0, 0.0),
        };
        let button = match subject.opener {
            Opener::Bar(..) => PointerButton::Primary,
            Opener::Context | Opener::Toolbar | Opener::Window(_) => PointerButton::Secondary,
        };
        let mut capture = self.click(subject, at, button);
        let mut open: Vec<LayerId> = Vec::new();
        let new_layer =
            |capture: &Capture, open: &[LayerId], what: &str| -> Result<LayerId, String> {
                let fresh: Vec<LayerId> = capture
                    .layers
                    .iter()
                    .map(|l| l.id)
                    .filter(|id| !open.contains(id))
                    .collect();
                match fresh.as_slice() {
                    [one] => Ok(*one),
                    _ => Err(format!("{what} opened {} new popups", fresh.len())),
                }
            };
        open.push(new_layer(&capture, &open, "the menu")?);
        for (row, caption) in path {
            let parent = capture
                .layers
                .iter()
                .find(|l| Some(&l.id) == open.last())
                .ok_or("the parent menu closed")?;
            let parent_id = parent.id;
            let parent_items = items_at(&subject.items, &path[..open.len() - 1]);
            capture = self.scroll_to(subject, capture, parent_id, parent_items, *row)?;
            let parent = capture
                .layers
                .iter()
                .find(|l| l.id == parent_id)
                .ok_or("the parent menu closed")?;
            let rows = parent.rows();
            let target = rows
                .get(*row)
                .ok_or_else(|| format!("submenu {caption:?}: no row {row}"))?;
            let at = target.center();
            self.frame(subject, vec![Event::PointerMoved(at)]);
            capture = self.settle(subject);
            open.push(new_layer(&capture, &open, &format!("submenu {caption:?}"))?);
        }
        Ok((capture, open))
    }
}

// ---- the measures --------------------------------------------------------------------------------------------------

/// What a row must show.
struct Expect {
    caption: String,
    shortcut: Option<String>,
    mark: Option<&'static str>,
    submenu: bool,
}

fn expected(items: &[MenuItem]) -> Vec<Expect> {
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Separator => None,
            MenuItem::Submenu { caption, .. } => Some(Expect {
                caption: caption.clone(),
                shortcut: None,
                mark: None,
                submenu: true,
            }),
            MenuItem::Command {
                caption,
                shortcut,
                check,
                ..
            } => Some(Expect {
                caption: caption.clone(),
                shortcut: shortcut.clone(),
                mark: match check {
                    Check::Toggle(true) => Some("✔"),
                    Check::Radio(true) => Some("●"),
                    _ => None,
                },
                submenu: false,
            }),
        })
        .collect()
}

/// The measures of one opened menu.
struct Report {
    /// The violations.
    bad: Vec<String>,
    /// The rows scrolled out of view (not drawn, so not measured).
    hidden: BTreeSet<usize>,
}

/// Measures the menu `layer` whose rows are `expect` (only the rows of `only` when given: a scrolled menu's second
/// look); `name` is its path in the messages.
fn measure(
    layer: &Layer,
    expect: &[Expect],
    style: &egui::Style,
    name: &str,
    only: Option<&BTreeSet<usize>>,
) -> Report {
    let mut bad = Vec::new();
    let mut hidden = BTreeSet::new();
    let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    let area = layer.rect;
    let margin = style.spacing.menu_margin;
    let inner = Rect::from_min_max(
        area.min + margin.left_top(),
        area.max - margin.right_bottom(),
    );
    let view = viewport(layer);
    let gap = min_gap(style);
    let icon_gap = style.spacing.icon_spacing;
    // The menu style's button padding (egui's `menu_style`).
    let padding = 2.0;
    let rows = layer.rows();
    let drawn = texts(&layer.shapes);
    let arrows = marks(&layer.shapes);
    let scroll_bars = bars(&layer.shapes, style.spacing.interact_size.y);
    if rows.len() != expect.len() {
        bad.push(format!(
            "{name}: {} items drawn, {} expected",
            rows.len(),
            expect.len()
        ));
        return Report { bad, hidden };
    }
    let on_screen = |r: Rect| {
        r.min.x >= screen.min.x - EPS
            && r.min.y >= screen.min.y - EPS
            && r.max.x <= screen.max.x + EPS
            && r.max.y <= screen.max.y + EPS
    };
    if !on_screen(area) {
        bad.push(format!(
            "{name}: the menu {:.1}×{:.1} at ({:.1}, {:.1})–({:.1}, {:.1}) is not on the {}×{} screen",
            area.width(),
            area.height(),
            area.min.x,
            area.min.y,
            area.max.x,
            area.max.y,
            SCREEN.x,
            SCREEN.y
        ));
    }
    if area.width() < MIN_MENU_WIDTH - EPS {
        bad.push(format!(
            "{name}: the menu is {:.1} wide, less than {MIN_MENU_WIDTH}",
            area.width()
        ));
    }
    let mut needed: f32 = 0.0;
    let mut widest = String::new();
    for (k, (row, want)) in rows.iter().zip(expect).enumerate() {
        if only.is_some_and(|o| !o.contains(&k)) {
            continue;
        }
        let item = format!("{name} ▸ {}", want.caption);
        if row.height() < style.spacing.interact_size.y - EPS {
            bad.push(format!(
                "{item}: the row is {:.1} high, less than the interact height {:.1}",
                row.height(),
                style.spacing.interact_size.y
            ));
        }
        if row.left() < inner.left() - EPS || row.right() > inner.right() + EPS {
            bad.push(format!(
                "{item}: the row {:.1}–{:.1} sticks out of the menu's inside {:.1}–{:.1}",
                row.left(),
                row.right(),
                inner.left(),
                inner.right()
            ));
        }
        // A row not wholly in the viewport is (partly) scrolled away: measured when the menu scrolls to it.
        if !view.expand(EPS).contains_rect(*row) {
            hidden.insert(k);
            continue;
        }
        if !on_screen(row.intersect(view)) {
            bad.push(format!(
                "{item}: the row {:.1}–{:.1} is off the {}×{} screen and the menu does not scroll to it",
                row.top(),
                row.bottom(),
                SCREEN.x,
                SCREEN.y
            ));
        }
        let in_row: Vec<&Text> = drawn
            .iter()
            .filter(|t| t.rect.center().y >= row.top() && t.rect.center().y <= row.bottom())
            .collect();
        let shown = || in_row.iter().map(|t| t.text.as_str()).collect::<Vec<_>>();
        for t in &in_row {
            let near = |b: &&Rect| b.expand2(vec2(icon_gap - EPS, -EPS)).intersects(t.rect);
            if let Some(bar) = scroll_bars.iter().find(near) {
                bad.push(format!(
                    "{item}: the scroll bar {:.1}–{:.1} covers or touches {:?} at {:.1}–{:.1}",
                    bar.left(),
                    bar.right(),
                    t.text,
                    t.rect.left(),
                    t.rect.right()
                ));
            }
        }
        let Some(label) = in_row.iter().find(|t| t.text == want.caption) else {
            bad.push(format!(
                "{item}: the label is not drawn in full (drawn: {:?})",
                shown()
            ));
            continue;
        };
        let galley = &label.galley;
        let label = label.rect;
        if galley.elided {
            bad.push(format!(
                "{item}: the label is cut off with an ellipsis ({:.1} wide in a row of {:.1})",
                label.width(),
                row.width()
            ));
        }
        if galley.rows.len() > 1 {
            bad.push(format!(
                "{item}: the label wraps onto {} lines",
                galley.rows.len()
            ));
        }
        if label.left() < row.left() - EPS || label.right() > row.right() + EPS {
            bad.push(format!(
                "{item}: the label {:.1}–{:.1} sticks out of its row {:.1}–{:.1}",
                label.left(),
                label.right(),
                row.left(),
                row.right()
            ));
        }
        let end = row.right().min(inner.right());
        let mut width = label.width() + 2.0 * padding;
        if let Some(mark) = want.mark {
            match in_row.iter().find(|t| t.text == mark) {
                None => bad.push(format!("{item}: its mark {mark:?} is not drawn")),
                Some(m) => {
                    let m = m.rect;
                    if m.right() > label.left() + EPS || m.left() < row.left() - EPS {
                        bad.push(format!(
                            "{item}: the mark {:.1}–{:.1} overlaps the label from {:.1} or leaves the row from {:.1}",
                            m.left(),
                            m.right(),
                            label.left(),
                            row.left()
                        ));
                    }
                    width += label.left() - m.left();
                }
            }
        }
        if let Some(shortcut) = &want.shortcut {
            match in_row.iter().find(|t| t.text == *shortcut) {
                None => bad.push(format!(
                    "{item}: its shortcut {shortcut:?} is not drawn in full (drawn: {:?})",
                    shown()
                )),
                Some(s) => {
                    let s = s.rect;
                    let between = s.left() - label.right();
                    if between < gap - EPS {
                        bad.push(format!(
                            "{item}: the shortcut {shortcut:?} is {between:.1} from the label, less than {gap:.1}",
                        ));
                    }
                    if s.right() > end + EPS {
                        bad.push(format!(
                            "{item}: the shortcut {shortcut:?} ends at {:.1}, past the row's end {end:.1}",
                            s.right(),
                        ));
                    }
                    width += gap + s.width();
                }
            }
        }
        if want.submenu {
            let arrow = arrows
                .iter()
                .find(|r| r.center().y >= row.top() && r.center().y <= row.bottom());
            match arrow {
                None => bad.push(format!("{item}: the submenu arrow is not drawn")),
                Some(a) => {
                    if a.left() < label.right() + icon_gap - EPS || a.right() > end + EPS {
                        bad.push(format!(
                            "{item}: the submenu arrow {:.1}–{:.1} overlaps the label ending at {:.1} or leaves the row ending at {end:.1}",
                            a.left(),
                            a.right(),
                            label.right(),
                        ));
                    }
                    width += icon_gap + style.spacing.icon_width;
                }
            }
        }
        if width > needed {
            needed = width;
            widest.clone_from(&want.caption);
        }
    }
    // A menu scrolls only when the screen cannot hold it.
    if only.is_none() && !hidden.is_empty() && area.height() < SCREEN.y - 1.0 {
        bad.push(format!(
            "{name}: the menu scrolls at {:.1} high ({} rows out of view) on a screen {} high",
            area.height(),
            hidden.len(),
            SCREEN.y
        ));
    }
    let total = needed + margin.sum().x;
    if area.width() < total - EPS {
        bad.push(format!(
            "{name}: the menu is {:.1} wide, {total:.1} needed by {widest:?}",
            area.width()
        ));
    }
    Report { bad, hidden }
}

// ---- the shots -----------------------------------------------------------------------------------------------------

/// Rasterises `prims` (egui's meshes, premultiplied, blended in gamma space as egui's renderers do) over `crop`
/// (points) into an RGBA image.
fn rasterise(
    prims: &[ClippedPrimitive],
    textures: &HashMap<TextureId, ColorImage>,
    ppp: f32,
    crop: Rect,
    background: Color32,
) -> (u32, u32, Vec<u8>) {
    let x0 = (crop.min.x * ppp).floor().max(0.0) as i64;
    let y0 = (crop.min.y * ppp).floor().max(0.0) as i64;
    let x1 = (crop.max.x * ppp).ceil().min(SCREEN.x * ppp) as i64;
    let y1 = (crop.max.y * ppp).ceil().min(SCREEN.y * ppp) as i64;
    let (w, h) = ((x1 - x0).max(1) as usize, (y1 - y0).max(1) as usize);
    let bg = background.to_array().map(|c| f32::from(c) / 255.0);
    let mut px = vec![bg; w * h];
    for prim in prims {
        let Primitive::Mesh(mesh) = &prim.primitive else {
            continue;
        };
        let texture = textures.get(&mesh.texture_id);
        let clip = prim.clip_rect;
        let cx0 = ((clip.min.x * ppp).floor() as i64).max(x0);
        let cy0 = ((clip.min.y * ppp).floor() as i64).max(y0);
        let cx1 = ((clip.max.x * ppp).ceil() as i64).min(x1);
        let cy1 = ((clip.max.y * ppp).ceil() as i64).min(y1);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            let p = v.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
            let area =
                (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let bx0 = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i64).max(cx0);
            let by0 = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i64).max(cy0);
            let bx1 = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i64).min(cx1);
            let by1 = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i64).min(cy1);
            for y in by0..by1 {
                for x in bx0..bx1 {
                    let (sx, sy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1].0 - sx) * (p[2].1 - sy) - (p[2].0 - sx) * (p[1].1 - sy)) / area;
                    let w1 = ((p[2].0 - sx) * (p[0].1 - sy) - (p[0].0 - sx) * (p[2].1 - sy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let mut c = [0.0f32; 4];
                    for (k, c) in c.iter_mut().enumerate() {
                        *c = (w0 * f32::from(v[0].color[k])
                            + w1 * f32::from(v[1].color[k])
                            + w2 * f32::from(v[2].color[k]))
                            / 255.0;
                    }
                    if let Some(t) = texture {
                        let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                        let vv = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                        let tx = ((u * t.size[0] as f32) as usize).min(t.size[0] - 1);
                        let ty = ((vv * t.size[1] as f32) as usize).min(t.size[1] - 1);
                        let texel = t.pixels[ty * t.size[0] + tx].to_array();
                        for (k, c) in c.iter_mut().enumerate() {
                            *c *= f32::from(texel[k]) / 255.0;
                        }
                    }
                    let dst = &mut px[(y - y0) as usize * w + (x - x0) as usize];
                    for k in 0..4 {
                        dst[k] = c[k] + dst[k] * (1.0 - c[3]);
                    }
                }
            }
        }
    }
    let rgba = px
        .iter()
        .flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    (w as u32, h as u32, rgba)
}

fn file_name(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Writes the opened menus of `capture` (the menu bar too) as a PNG under `dir`.
fn shot(h: &Harness, capture: &Capture, dir: &std::path::Path, scope: Scope, name: &str) {
    if !h.shots {
        return;
    }
    // The menus draw text only: the font atlas is their one texture.
    let textures = HashMap::from([(TextureId::default(), h.ctx.fonts(|f| f.image()))]);
    let mut crop = Rect::NOTHING;
    for layer in &capture.layers {
        crop = crop.union(layer.rect);
    }
    for (_, r) in &capture.titles {
        crop = crop.union(*r);
    }
    let crop = crop.expand(12.0);
    let prims = h.ctx.tessellate(capture.output.shapes.clone(), h.ppp);
    let background = h.app.palette.bg;
    let (w, height, rgba) = rasterise(&prims, &textures, h.ppp, crop, background);
    let png = hikasa_session::clipboard::encode_png_rgba(w, height, &rgba).expect("png");
    let folder = dir
        .join(format!("{}x", h.ppp))
        .join(file_name(scope.type_title()));
    std::fs::create_dir_all(&folder).expect("shots folder");
    std::fs::write(folder.join(format!("{}.png", file_name(name))), png).expect("shot");
}

// ---- the test ------------------------------------------------------------------------------------------------------

/// Opens and measures every menu and submenu of `subjects` at one scale; returns the violations and the counts of
/// menus and items measured.
fn measure_all(ppp: f32, shots: Option<&std::path::Path>) -> (Vec<String>, usize, usize) {
    let mut h = Harness::new(ppp, shots.is_some());
    let tables = h.app.core.tables().clone();
    let style = h.ctx.global_style();
    let mut bad = Vec::new();
    let (mut menus, mut items) = (0, 0);
    for scope in Scope::VIEWS {
        for subject in subjects(&h.world, &tables, scope) {
            let mut paths = vec![Vec::new()];
            submenu_paths(&subject.items, &[], &mut paths);
            for path in paths {
                let name = std::iter::once(subject.name.clone())
                    .chain(path.iter().map(|(_, c)| c.clone()))
                    .collect::<Vec<_>>()
                    .join(" ▸ ");
                let name = format!("{name} (×{ppp})");
                match h.open(&subject, &path) {
                    Err(e) => bad.push(format!("{name}: {e}")),
                    Ok((capture, open)) => {
                        let last = open.last().copied();
                        let layer = capture.layers.iter().find(|l| Some(l.id) == last);
                        let want = expected(items_at(&subject.items, &path));
                        menus += 1;
                        items += want.len();
                        let Some(layer) = layer else {
                            bad.push(format!("{name}: the menu closed"));
                            continue;
                        };
                        let layer_id = layer.id;
                        let report = measure(layer, &want, &style, &name, None);
                        bad.extend(report.bad);
                        if let Some(dir) = shots {
                            shot(&h, &capture, dir, scope, &name);
                        }
                        // A scrolling menu: its last hidden row scrolled into view, the hidden rows measured.
                        if let Some(&last) = report.hidden.last() {
                            let sub = items_at(&subject.items, &path);
                            match h.scroll_to(&subject, capture, layer_id, sub, last) {
                                Err(e) => bad.push(format!("{name}: {e}")),
                                Ok(capture) => {
                                    let name = format!("{name} scrolled");
                                    if let Some(layer) =
                                        capture.layers.iter().find(|l| l.id == layer_id)
                                    {
                                        let again = measure(
                                            layer,
                                            &want,
                                            &style,
                                            &name,
                                            Some(&report.hidden),
                                        );
                                        bad.extend(again.bad);
                                        let left: Vec<_> =
                                            report.hidden.intersection(&again.hidden).collect();
                                        if !left.is_empty() {
                                            bad.push(format!("{name}: rows {left:?} never show"));
                                        }
                                    }
                                    if let Some(dir) = shots {
                                        shot(&h, &capture, dir, scope, &name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    (bad, menus, items)
}

/// The maintainer's request: every menu has an adequate size, at every scale.
#[test]
fn every_menu_has_an_adequate_size() {
    let shots = std::env::var_os("HIKASA_MENU_SHOTS").map(PathBuf::from);
    let mut bad = Vec::new();
    let mut counts = Vec::new();
    for ppp in SCALES {
        let (b, menus, items) = measure_all(ppp, shots.as_deref());
        bad.extend(b);
        counts.push((ppp, menus, items));
    }
    for (ppp, menus, items) in &counts {
        assert!(
            *menus > 100 && *items > 1000,
            "×{ppp}: only {menus} menus and {items} items measured:\n{}",
            bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
        );
    }
    if std::env::var_os("HIKASA_MENU_COUNTS").is_some() {
        eprintln!("menus measured (scale, menus, items): {counts:?}");
    }
    assert!(
        bad.is_empty(),
        "{} menu size violations:\n{}",
        bad.len(),
        bad.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The picture view's context menu as the window draws it (`ui/picture.rs` shows it in an area of its own, not in
/// egui's menu popup): measured as the others, at every scale.
#[test]
fn the_picture_view_menu_has_an_adequate_size() {
    let shots = std::env::var_os("HIKASA_MENU_SHOTS").map(PathBuf::from);
    let mut bad = Vec::new();
    for ppp in SCALES {
        let mut h = Harness::new(ppp, shots.is_some());
        let style = h.ctx.global_style();
        let (l, r) = crate::tests_picture_open::pair(&h.world);
        let view = h.world.open(&mut h.app.core, &l, &r);
        let mut subject = Subject {
            scope: Scope::Picture,
            name: format!("Picture Compare ▸ the difference pane's menu (×{ppp})"),
            opener: Opener::Window(Pos2::ZERO),
            items: Vec::new(),
        };
        for _ in 0..3 {
            h.world.settle(&mut h.app.core);
            h.settle(&subject);
        }
        let pane = match h.app.core.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(crate::window::ViewModel::Picture(m)) => {
                m.slots()
                    .iter()
                    .find(|x| x.pane == hikasa_picture::Pane::Difference)
                    .expect("the difference pane")
                    .picture
            }
            _ => panic!("not a picture view"),
        };
        subject.opener = Opener::Window(pos2(
            (pane.x + pane.w / 2.0) as f32,
            (pane.y + pane.h / 2.0) as f32,
        ));
        let (capture, open) = h
            .open(&subject, &[])
            .unwrap_or_else(|e| panic!("{}: does not stay open ({e})", subject.name));
        subject.items = h
            .app
            .pictures
            .get(&view)
            .map(|st| st.menu_items.clone())
            .unwrap_or_default();
        assert!(subject.items.len() > 5, "the menu showed no items");
        let layer = capture
            .layers
            .iter()
            .find(|l| Some(l.id) == open.last().copied())
            .expect("the menu's layer");
        let report = measure(
            layer,
            &expected(&subject.items),
            &style,
            &subject.name,
            None,
        );
        bad.extend(report.bad);
        assert!(report.hidden.is_empty(), "the menu scrolls at 720 points");
        if let Some(dir) = &shots {
            shot(&h, &capture, dir, Scope::Picture, &subject.name);
        }
    }
    assert!(
        bad.is_empty(),
        "{} menu size violations:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
