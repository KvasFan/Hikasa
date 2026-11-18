//! Drawing of the Picture Compare view (M2 W13b, §1.13.10…§1.13.13): the only owner of textures in the GUI (D-373):
//! the tiles and overviews the model uploads (≤ 8 a frame, the 64 MiB budget counted by the model), the window-painted
//! checkerboard (one repeating texture anchored at the area's top-left corner, 8 physical pixels a cell), the
//! background, the pixel grid at ≥ 1,600 %, the offset outline while dragging, the probe's crosshair, the File Info
//! headers, the Tolerance spinner and the Blend Percentage slider; pointer and keys become model calls.

use std::collections::HashMap;

use egui::{
    Align2, Color32, ColorImage, CornerRadius, Event, FontId, Id, Key, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode, Ui, pos2, vec2,
};
use hikasa_picture::{DiffMode, Pane, PictureColors, Rgba8, RgbaTile};
use hikasa_session::Service;
use hikasa_session::ipc::ViewId;

use crate::commands::{Area, CommandId};
use crate::picture::tiles::{TextureSink, TileKey};
use crate::picture::viewport::PaneRect;
use crate::picture::{PicturePhase, PictureViewModel, Split};
use crate::ui::theme::Palette;
use crate::window::{CommandArg, Invocation};

/// The File Info header's height (the fields' row and a line of facts).
const HEADER_H: f64 = 40.0;
/// A header row's height.
const LINE: f32 = 21.0;
/// The strip with the Tolerance spinner and the Blend Percentage slider.
const STRIP_H: f32 = 26.0;
/// The scroll bars' width.
const BAR: f32 = 9.0;
/// The grip of a splitter (points across).
const SPLITTER: f32 = 6.0;

/// The textures of a picture view and its pointer state.
#[derive(Default)]
pub struct PictureUi {
    textures: HashMap<TileKey, TextureHandle>,
    checker: Option<([Rgba8; 2], TextureHandle)>,
    menu: Option<(Pane, Pos2)>,
    /// A drag runs (a press in a pane).
    dragging: bool,
    /// The model the textures belong to (`PictureViewModel::instance`).
    instance: u64,
    /// The items of the context menu last drawn (tests).
    #[cfg(test)]
    pub menu_items: Vec<crate::menus::MenuItem>,
    /// The window's picture clock when the view was last shown (the budget evicts the least recently shown first).
    pub last_shown: u64,
    /// The button of a pane's press is still down (a press starts one drag; after Esc none until it is released).
    held: bool,
}

/// The window's texture owner as the model's sink.
pub struct Sink<'a> {
    ctx: &'a egui::Context,
    textures: &'a mut HashMap<TileKey, TextureHandle>,
}

impl PictureUi {
    /// The textures the window holds for the view (tests).
    #[cfg(test)]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }
}

impl<'a> Sink<'a> {
    /// The sink of a view's textures.
    pub fn new(ctx: &'a egui::Context, st: &'a mut PictureUi) -> Sink<'a> {
        Sink {
            ctx,
            textures: &mut st.textures,
        }
    }
}

impl TextureSink for Sink<'_> {
    fn upload(&mut self, key: TileKey, image: &RgbaTile) {
        let color = ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.rgba,
        );
        // Pixels stay sharp when zoomed in; the overview is linear both ways (§1.13.11).
        let options = TextureOptions {
            magnification: if key.overview {
                TextureFilter::Linear
            } else {
                TextureFilter::Nearest
            },
            minification: TextureFilter::Linear,
            wrap_mode: TextureWrapMode::ClampToEdge,
            mipmap_mode: None,
        };
        let name = format!(
            "picture-{}-{}-{}-{}-{}-{}",
            key.pane, key.generation, key.level, key.x, key.y, key.overview
        );
        let handle = self.ctx.load_texture(name, color, options);
        self.textures.insert(key, handle);
    }

    fn evict(&mut self, key: TileKey) {
        self.textures.remove(&key);
    }
}

fn color(c: Rgba8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn rect_of(r: PaneRect) -> Rect {
    Rect::from_min_size(pos2(r.x as f32, r.y as f32), vec2(r.w as f32, r.h as f32))
}

/// The checkerboard texture: 2 × 2 cells of one texel each, repeated (a cell is 8 physical pixels on screen).
fn checker(ctx: &egui::Context, st: &mut PictureUi, colors: &PictureColors) -> egui::TextureId {
    let want = [colors.checker_light, colors.checker_dark];
    if let Some((have, handle)) = &st.checker
        && *have == want
    {
        return handle.id();
    }
    let (l, d) = (color(want[0]), color(want[1]));
    let image = ColorImage::new([2, 2], vec![l, d, d, l]);
    let options = TextureOptions {
        magnification: TextureFilter::Nearest,
        minification: TextureFilter::Nearest,
        wrap_mode: TextureWrapMode::Repeat,
        mipmap_mode: None,
    };
    let handle = ctx.load_texture("picture-checkerboard", image, options);
    let id = handle.id();
    st.checker = Some((want, handle));
    id
}

/// The window's builder of a pane's context menu: the area, the predicates' context, the view's checks.
pub type PaneMenu<'a> = dyn Fn(
        Area,
        &crate::commands::Ctx,
        &dyn Fn(CommandId) -> Option<bool>,
    ) -> Vec<crate::menus::MenuItem>
    + 'a;

/// Draws the view and turns input into model calls; returns the commands to run.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut PictureViewModel,
    st: &mut PictureUi,
    service: &Service,
    pal: &Palette,
    budget: u64,
    keys: bool,
    menu: &PaneMenu<'_>,
    ctx_now: crate::commands::Ctx,
) -> Vec<Invocation> {
    let mut run: Vec<Invocation> = Vec::new();
    // A new model in the tab (the next pair, a session reopened): the old one's textures are nobody's.
    if st.instance != m.instance() {
        st.textures.clear();
        st.menu = None;
        st.dragging = false;
        st.instance = m.instance();
    }
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let ctx = ui.ctx().clone();
    m.set_ppp(f64::from(ctx.pixels_per_point()));
    let colors = m.colors();

    // The strip: the Tolerance spinner, and the Blend Percentage slider in Blend mode.
    let strip = Rect::from_min_size(full.min, vec2(full.width(), STRIP_H));
    if m.full_screen.is_none() {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(strip.shrink2(vec2(6.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut tolerance = u32::from(m.options.tolerance);
        child.label("Tolerance");
        if child
            .add(egui::DragValue::new(&mut tolerance).range(0..=255))
            .changed()
        {
            run.push(Invocation::with(
                CommandId::Tolerance,
                CommandArg::Number(tolerance),
            ));
        }
        if let DiffMode::Blend { percent } = m.shown_mode() {
            child.separator();
            child.label("Blend");
            let mut p = u32::from(percent);
            if child
                .add(egui::Slider::new(&mut p, 0..=100).suffix(" %"))
                .changed()
            {
                run.push(Invocation::with(
                    CommandId::BlendPercent,
                    CommandArg::Number(p),
                ));
            }
        }
    }
    let content = if m.full_screen.is_some() {
        full
    } else {
        Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max)
    };
    let slots = m.arrange(
        PaneRect::new(
            f64::from(content.left()),
            f64::from(content.top()),
            f64::from(content.width()),
            f64::from(content.height()),
        ),
        HEADER_H,
    );
    // The tiles of this frame: renders, uploads, evictions.
    let repaint = {
        let mut sink = Sink::new(&ctx, st);
        m.frame(service, budget, &mut sink)
    };
    if repaint {
        ctx.request_repaint();
    }
    let painter = ui.painter_at(full);
    let checker_id = checker(&ctx, st, &colors);
    let small = FontId::proportional(11.0);
    let probe = m.probe().copied();
    let scale = m.scale();
    for slot in &slots {
        let pane = slot.pane;
        let rect = rect_of(slot.picture);
        let pane_painter = painter.with_clip_rect(rect);
        // The scroll bars' tracks and knobs (all panes move together).
        let bars: Vec<(usize, Rect, Rect, f64)> = m
            .scrollbars(pane)
            .iter()
            .enumerate()
            .filter_map(|(axis, bar)| {
                let (start, len) = (*bar)?;
                let (track, knob) = bar_rects(rect, axis, start, len);
                Some((axis, track, knob, len))
            })
            .collect();
        // Input of the pane, registered before the bars: a bar lies on top of the pane and wins egui's hit test, so a
        // press on a bar scrolls and never starts the pane's drag (W13b review finding 2).
        let response = ui.interact(
            rect,
            Id::unique(("hikasa-picture", view.0, pane_word(pane))),
            Sense::click_and_drag(),
        );
        if keys
            && response.secondary_clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            m.focus = pane;
            st.menu = Some((pane, p));
        }
        // A drag starts on the press itself, from where the button went down (the whole distance counts; Esc returns
        // to the offset of that moment): never again while the same press holds (W13b review finding 3).
        if keys && response.is_pointer_button_down_on() && !st.held {
            st.held = true;
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(p) = origin
                && !bars.iter().any(|(_, track, _, _)| track.contains(p))
                && !ui.input(|i| i.pointer.secondary_down())
            {
                let middle = ui.input(|i| i.pointer.middle_down());
                m.press((f64::from(p.x), f64::from(p.y)), middle);
                st.dragging = true;
            }
        } else if keys && response.clicked() {
            m.focus = pane;
        }
        // Header: a side's path field (Enter reloads that side), *Browse…* and the format picker, then its facts; the
        // difference pane's area, mode and verdict (§1.13.13).
        if slot.header.h > 0.0 {
            let h = rect_of(slot.header);
            painter.rect_filled(h, CornerRadius::ZERO, pal.chrome);
            let lines = m.header_lines(pane, service);
            let side = match pane {
                Pane::Left => Some(hikasa_core::Side::Left),
                Pane::Right => Some(hikasa_core::Side::Right),
                Pane::Difference => None,
            };
            let first = match side {
                Some(side) => {
                    header_fields(ui, view, m, service, side, h, lines.first());
                    1
                }
                _ => 0,
            };
            for (k, line) in lines.iter().enumerate().skip(first) {
                let failed = k == 1 && side.is_some_and(|s| m.side_error(s).is_some());
                painter.text(
                    pos2(h.left() + 6.0, h.top() + 2.0 + LINE * k as f32),
                    Align2::LEFT_TOP,
                    line,
                    small.clone(),
                    if failed { pal.error } else { pal.fg },
                );
            }
            if m.focus == pane {
                painter.line_segment(
                    [h.left_bottom(), h.right_bottom()],
                    Stroke::new(2.0, pal.accent),
                );
            }
        }
        // Background outside the area, the checkerboard (or the background) inside it.
        pane_painter.rect_filled(rect, CornerRadius::ZERO, color(colors.background));
        if let Some((x0, y0, x1, y1)) = m.area_on_screen(pane) {
            let area = Rect::from_min_max(pos2(x0 as f32, y0 as f32), pos2(x1 as f32, y1 as f32));
            if m.checkerboard
                && let Some(((ax, ay), cell)) = m.checker_anchor(pane)
            {
                let shown = area.intersect(rect);
                if shown.is_positive() {
                    let c = cell as f32;
                    let uv = Rect::from_min_max(
                        pos2(
                            (shown.left() - ax as f32) / c,
                            (shown.top() - ay as f32) / c,
                        ),
                        pos2(
                            (shown.right() - ax as f32) / c,
                            (shown.bottom() - ay as f32) / c,
                        ),
                    );
                    // Texel (u, v) covers cells u..u+1: 2 texels per 2 cells.
                    let uv = Rect::from_min_max(
                        pos2(uv.min.x / 2.0, uv.min.y / 2.0),
                        pos2(uv.max.x / 2.0, uv.max.y / 2.0),
                    );
                    pane_painter.image(checker_id, shown, uv, Color32::WHITE);
                }
            }
        }
        // The overview, then the tiles (older generations first, clipped by the model).
        for item in m.draw_list(pane) {
            let Some(handle) = st.textures.get(&item.key) else {
                continue;
            };
            let (x0, y0, x1, y1) = item.rect;
            let (u0, v0, u1, v1) = item.uv;
            pane_painter.image(
                handle.id(),
                Rect::from_min_max(pos2(x0 as f32, y0 as f32), pos2(x1 as f32, y1 as f32)),
                Rect::from_min_max(pos2(u0 as f32, v0 as f32), pos2(u1 as f32, v1 as f32)),
                Color32::WHITE,
            );
        }
        // The pixel grid at ≥ 1,600 %.
        if let Some((xs, ys)) = m.grid_lines(pane) {
            let c = colors.checker_dark;
            let grid = Color32::from_rgba_unmultiplied(c.r, c.g, c.b, 64);
            let stroke = Stroke::new(1.0 / ctx.pixels_per_point(), grid);
            for x in xs {
                pane_painter.vline(x as f32, rect.y_range(), stroke);
            }
            for y in ys {
                pane_painter.hline(rect.x_range(), y as f32, stroke);
            }
        }
        // The right picture's outline at the new offset while dragging.
        if pane == Pane::Difference
            && let Some((x0, y0, x1, y1)) = m.offset_outline()
        {
            pane_painter.rect_stroke(
                Rect::from_min_max(pos2(x0 as f32, y0 as f32), pos2(x1 as f32, y1 as f32)),
                CornerRadius::ZERO,
                Stroke::new(1.5, pal.accent),
                StrokeKind::Inside,
            );
        }
        // The probe's crosshair in every pane.
        if let Some(p) = probe {
            let area = m.area();
            let at = m.viewport.to_screen(
                slot.picture,
                scale,
                area,
                (f64::from(p.area_x) + 0.5, f64::from(p.area_y) + 0.5),
            );
            let stroke = Stroke::new(1.0, pal.accent.gamma_multiply(0.6));
            pane_painter.hline(rect.x_range(), at.1 as f32, stroke);
            pane_painter.vline(at.0 as f32, rect.y_range(), stroke);
        }
        // The scroll bars: a click or a drag puts the knob's middle under the pointer, in every pane.
        for (axis, track, knob, len) in bars {
            pane_painter.rect_filled(track, CornerRadius::ZERO, pal.chrome.gamma_multiply(0.7));
            pane_painter.rect_filled(knob, CornerRadius::same(3), pal.knob);
            let r = ui.interact(
                track,
                Id::unique(("hikasa-picture-bar", view.0, pane_word(pane), axis)),
                Sense::click_and_drag(),
            );
            if keys
                && (r.dragged() || r.clicked() || r.is_pointer_button_down_on())
                && let Some(p) = r.interact_pointer_pos()
            {
                let at = if axis == 0 {
                    f64::from((p.x - track.left()) / track.width().max(1.0))
                } else {
                    f64::from((p.y - track.top()) / track.height().max(1.0))
                };
                m.scroll_to(pane, axis, at - len / 2.0);
            }
        }
        if m.phase() != PicturePhase::Ready {
            let text = match m.phase() {
                PicturePhase::Loading => "Decoding…",
                PicturePhase::Comparing => "Comparing…",
                _ => m.error().unwrap_or("The pictures could not be compared."),
            };
            pane_painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(13.0),
                pal.muted,
            );
        }
    }
    // The splitters (registered after the panes: on top of them), their positions kept in the view type's layout.
    for (split, (x0, y0, x1, y1), vertical) in m.splitters() {
        let line = Rect::from_min_max(pos2(x0 as f32, y0 as f32), pos2(x1 as f32, y1 as f32));
        painter.line_segment([line.min, line.max], Stroke::new(1.0, pal.chrome));
        let grip = if vertical {
            line.expand2(vec2(SPLITTER / 2.0, 0.0))
        } else {
            line.expand2(vec2(0.0, SPLITTER / 2.0))
        };
        let r = ui.interact(
            grip,
            Id::unique(("hikasa-picture-split", view.0, split == Split::Panes)),
            Sense::drag(),
        );
        if r.hovered() || r.dragged() {
            ctx.set_cursor_icon(if vertical {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
        }
        if keys
            && r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            m.drag_split(split, (f64::from(p.x), f64::from(p.y)));
        }
    }
    // Drags, the pointer, the wheel and pinches.
    let (pointer, down, events, modifiers) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.any_down(),
            i.events.clone(),
            i.modifiers,
        )
    });
    if !down {
        st.held = false;
    }
    if st.dragging {
        if down {
            if let Some(p) = pointer {
                m.drag_to(service, (f64::from(p.x), f64::from(p.y)));
            }
            ctx.request_repaint();
        } else {
            m.release();
            st.dragging = false;
        }
    } else if let Some(p) = pointer {
        if content.contains(p) {
            m.pointer_moved((f64::from(p.x), f64::from(p.y)));
        } else {
            m.pointer_left();
        }
    }
    // Nothing reaches the panes under a dialog (W13b review finding 6).
    if keys && let Some(p) = pointer.filter(|p| content.contains(*p)) {
        let at = (f64::from(p.x), f64::from(p.y));
        for event in &events {
            match event {
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } => {
                    let k = match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => crate::picture::WHEEL_NOTCH,
                        egui::MouseWheelUnit::Page => f64::from(content.height()),
                    };
                    let d = (f64::from(delta.x) * k, f64::from(delta.y) * k);
                    m.wheel(at, d, modifiers.ctrl || modifiers.command, modifiers.shift);
                }
                Event::Zoom(factor) => m.pinch(at, f64::from(*factor)),
                _ => {}
            }
        }
    }
    // Keys: nudges, pages, Esc (no text field has the keyboard).
    if keys && !ctx.text_edit_focused() && !st.dragging {
        let pressed = |key: Key| ui.input(|i| i.key_pressed(key));
        let ctrl = modifiers.ctrl || modifiers.command;
        for (key, dx, dy) in [
            (Key::ArrowLeft, -1, 0),
            (Key::ArrowRight, 1, 0),
            (Key::ArrowUp, 0, -1),
            (Key::ArrowDown, 0, 1),
        ] {
            if pressed(key) && !modifiers.shift && !modifiers.alt {
                m.nudge(service, dx, dy, ctrl);
            }
        }
        if pressed(Key::PageUp) {
            m.page(false, None);
        }
        if pressed(Key::PageDown) {
            m.page(true, None);
        }
        if pressed(Key::Home) && !ctrl {
            m.page(false, Some(false));
        }
        if pressed(Key::End) && !ctrl {
            m.page(false, Some(true));
        }
    }
    if keys && ui.input(|i| i.key_pressed(Key::Escape)) && m.escape(service) {
        st.dragging = false;
    }
    // The context menu (UI-005).
    if let Some((pane, at)) = st.menu {
        let id = Id::unique(("hikasa-picture-menu", view.0));
        let mut open = true;
        let area = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let mut c = ctx_now;
                    m.ctx_into(&mut c);
                    let items = menu(Area::PicturePane(pane), &c, &|id| m.checked(id));
                    #[cfg(test)]
                    {
                        st.menu_items = items.clone();
                    }
                    let mut hint = None;
                    let before = run.len();
                    crate::ui::menus::items(ui, &items, &mut run, &mut hint);
                    if run.len() > before {
                        open = false;
                    }
                });
            });
        if area.response.clicked_elsewhere() || ui.input(|i| i.key_pressed(Key::Escape)) {
            open = false;
        }
        if !open {
            st.menu = None;
        }
    }
    if m.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    run
}

/// A side's header row (§1.13.8): the path field (Enter reloads the side with the typed spec), *Browse…* (the
/// window's browse dialog) and the format picker (*Auto* or an enabled Picture format; a change decodes again).
fn header_fields(
    ui: &mut Ui,
    view: ViewId,
    m: &mut PictureViewModel,
    service: &Service,
    side: hikasa_core::Side,
    h: Rect,
    shown: Option<&String>,
) {
    let k = usize::from(side != hikasa_core::Side::Left);
    let row = Rect::from_min_size(
        pos2(h.left() + 4.0, h.top() + 1.0),
        vec2(h.width() - 8.0, LINE - 2.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(("picture-header", view.0, k)),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 6.0;
    let current = shown.cloned().unwrap_or_default();
    let mut text = m.header_paths[k].clone().unwrap_or_else(|| current.clone());
    let width = (row.width() - 170.0).max(60.0);
    let edit = ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(FontId::monospace(11.0))
            .desired_width(width),
    );
    mark(ui.ctx(), header_id(view, k, "path"), edit.rect);
    if edit.changed() || edit.has_focus() {
        m.header_paths[k] = Some(text.clone());
    }
    if edit.lost_focus() {
        let accepted = ui.input(|i| i.key_pressed(Key::Enter));
        let typed = m.header_paths[k].take();
        if accepted
            && let Some(typed) = typed.filter(|t| !t.trim().is_empty() && t.trim() != current)
        {
            m.set_spec(service, side, std::ffi::OsString::from(typed.trim()));
        }
    }
    let browse = ui.add(egui::Button::new(egui::RichText::new("Browse…").size(11.0)).small());
    mark(ui.ctx(), header_id(view, k, "browse"), browse.rect);
    if browse.clicked() {
        m.browse(side);
    }
    let format = m.side_format(side).unwrap_or("Auto").to_owned();
    let names = m.format_names().to_vec();
    let picker = ui.menu_button(egui::RichText::new(format).size(11.0), |ui| {
        let auto = ui.button("Auto");
        mark(ui.ctx(), header_id(view, k, "Auto"), auto.rect);
        if auto.clicked() {
            m.set_side_format(service, side, None);
            ui.close();
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for name in &names {
                    let item = ui.button(name);
                    mark(ui.ctx(), header_id(view, k, name), item.rect);
                    if item.clicked() {
                        m.set_side_format(service, side, Some(name.clone()));
                        ui.close();
                    }
                }
            });
    });
    mark(ui.ctx(), header_id(view, k, "format"), picker.response.rect);
}

/// The id under which a header widget's place is kept (`what`: `path`, `browse`, `format`, a format's name).
pub fn header_id(view: ViewId, k: usize, what: &str) -> Id {
    Id::unique(("hk-picture-header", view.0, k, what))
}

/// Keeps a widget's place for a frame (tests click there; the window reads nothing back).
pub fn mark(ctx: &egui::Context, id: Id, rect: Rect) {
    ctx.memory_mut(|m| m.data.insert_temp(id, rect));
}

/// A scroll bar's track and knob in a pane (axis 0 horizontal at the bottom, 1 vertical at the right); `start` and
/// `len` are fractions of the area.
fn bar_rects(rect: Rect, axis: usize, start: f64, len: f64) -> (Rect, Rect) {
    if axis == 0 {
        let track = Rect::from_min_max(
            pos2(rect.left(), rect.bottom() - BAR),
            pos2(rect.right() - BAR, rect.bottom()),
        );
        let knob = Rect::from_min_max(
            pos2(
                track.left() + track.width() * start as f32,
                track.top() + 1.0,
            ),
            pos2(
                track.left() + track.width() * (start + len) as f32,
                track.bottom() - 1.0,
            ),
        );
        (track, knob)
    } else {
        let track = Rect::from_min_max(
            pos2(rect.right() - BAR, rect.top()),
            pos2(rect.right(), rect.bottom() - BAR),
        );
        let knob = Rect::from_min_max(
            pos2(
                track.left() + 1.0,
                track.top() + track.height() * start as f32,
            ),
            pos2(
                track.right() - 1.0,
                track.top() + track.height() * (start + len) as f32,
            ),
        );
        (track, knob)
    }
}

fn pane_word(pane: Pane) -> &'static str {
    crate::picture::pane_word(pane)
}

/// The Picture Compare Info dialog's body.
pub fn info(ui: &mut Ui, d: &crate::picture::info::PictureInfoDialog, pal: &Palette) {
    egui::ScrollArea::vertical()
        .max_height(420.0)
        .show(ui, |ui| {
            for section in d.sections() {
                ui.label(egui::RichText::new(&section.title).strong().color(pal.fg));
                egui::Grid::new(("picture-info", section.title.as_str()))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for row in &section.rows {
                            ui.label(egui::RichText::new(&row.label).color(pal.muted));
                            ui.label(egui::RichText::new(row.values.join("  ")).color(pal.fg));
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
            }
        });
    if let Some(message) = &d.message {
        ui.label(egui::RichText::new(message).color(pal.muted));
    }
}

/// The Picture and Format tabs of a picture session (M2 W13b, §1.13.18): fields as words, checked by core.
pub fn session_tab(ui: &mut Ui, tab: &mut dyn std::any::Any) {
    if let Some(t) = tab.downcast_mut::<crate::picture_tabs::PictureTab>() {
        let f = &mut t.fields;
        ui.label(egui::RichText::new("Mode").strong());
        for (word, caption) in crate::picture_tabs::MODES {
            ui.radio_value(&mut f.mode, word.to_owned(), caption);
        }
        ui.add_space(6.0);
        egui::Grid::new("hk-picture-tab")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Tolerance (0–255)");
                ui.add(egui::TextEdit::singleline(&mut f.tolerance).desired_width(80.0));
                ui.end_row();
                ui.label("Blend percentage (0–100)");
                ui.add(egui::TextEdit::singleline(&mut f.blend).desired_width(80.0));
                ui.end_row();
                ui.label("Tone map");
                ui.horizontal(|ui| {
                    for (word, caption) in crate::picture_tabs::TONE_MAPS {
                        ui.radio_value(&mut f.tone_map, word.to_owned(), caption);
                    }
                });
                ui.end_row();
                ui.label("Maximum megapixels (1–4096)");
                ui.add(egui::TextEdit::singleline(&mut f.max_megapixels).desired_width(80.0));
                ui.end_row();
            });
        ui.checkbox(&mut f.ignore_unimportant, "Ignore unimportant differences");
        ui.checkbox(&mut f.auto_scale, "Auto Scale");
    } else if let Some(t) = tab.downcast_mut::<crate::picture_tabs::PictureFormatTab>() {
        if t.disabled {
            ui.label("The defaults of a type hold no inputs.");
            return;
        }
        egui::Grid::new("hk-picture-format")
            .num_columns(2)
            .show(ui, |ui| {
                for (label, value) in [
                    ("Left format", &mut t.fields.left),
                    ("Right format", &mut t.fields.right),
                ] {
                    ui.label(label);
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .hint_text("by the name")
                            .desired_width(240.0),
                    );
                    ui.end_row();
                }
            });
    }
}
