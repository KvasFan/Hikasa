//! The scroll bars of the custom-painted views. Their vertical positions are row indices kept in `f64` (a
//! 1,000,000-row view is far beyond what an `f32` pixel offset resolves), so they do not use egui's
//! `ScrollArea`; the bar maps rows to its track and back. The folder view's sides scroll horizontally in points
//! (D-234).

use egui::{CornerRadius, Id, Rect, Sense, Ui, Vec2};

use crate::ui::theme::Palette;

/// Smallest knob height (px).
const MIN_KNOB: f32 = 18.0;

/// Draws the bar in `rect` for `total` rows of which `visible` fit on screen, the first at `top`; returns the
/// new first row when the user dragged the knob or clicked the track (a page up or down).
pub fn vertical(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    total: f64,
    visible: f64,
    top: f64,
    pal: &Palette,
) -> Option<f64> {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.vline(
        rect.left(),
        rect.y_range(),
        egui::Stroke::new(1.0, pal.border),
    );
    let max_top = (total - visible).max(0.0);
    let track = rect.shrink2(Vec2::new(2.0, 2.0));
    let knob_h = if total > 0.0 {
        (track.height() * (visible / total).min(1.0) as f32).max(MIN_KNOB)
    } else {
        track.height()
    }
    .min(track.height());
    let travel = (track.height() - knob_h).max(0.0);
    let knob_y = if max_top > 0.0 {
        track.top() + travel * (top / max_top).clamp(0.0, 1.0) as f32
    } else {
        track.top()
    };
    let knob = Rect::from_min_size(
        egui::pos2(track.left(), knob_y),
        Vec2::new(track.width(), knob_h),
    );
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let hovered = response.hovered() || response.dragged();
    let colour = if hovered { pal.muted } else { pal.knob };
    painter.rect_filled(knob, CornerRadius::same(4), colour);
    if max_top <= 0.0 {
        return None;
    }
    let pointer = response.interact_pointer_pos()?;
    let grab_id = id.with("grab");
    if response.drag_started() || response.clicked() {
        if knob.contains(pointer) {
            ui.data_mut(|d| d.insert_temp(grab_id, pointer.y - knob.top()));
        } else {
            // A click on the track pages towards the pointer.
            ui.data_mut(|d| d.remove::<f32>(grab_id));
            let page = (visible - 1.0).max(1.0);
            let next = if pointer.y < knob.top() {
                top - page
            } else {
                top + page
            };
            return Some(next.clamp(0.0, max_top));
        }
    }
    if response.dragged()
        && travel > 0.0
        && let Some(grab) = ui.data(|d| d.get_temp::<f32>(grab_id))
    {
        let y = pointer.y - grab - track.top();
        let frac = f64::from((y / travel).clamp(0.0, 1.0));
        return Some(frac * max_top);
    }
    None
}

/// A horizontal bar in `rect` for content `total` points wide of which `visible` show, scrolled by `left` points;
/// returns the new offset when the user dragged the knob or clicked the track (a page left or right). The folder
/// view's sides scroll together with it (D-234).
pub fn horizontal(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    total: f32,
    visible: f32,
    left: f32,
    pal: &Palette,
) -> Option<f32> {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, pal.border),
    );
    let max_left = (total - visible).max(0.0);
    let track = rect.shrink2(Vec2::new(2.0, 2.0));
    let knob_w = if total > 0.0 {
        (track.width() * (visible / total).min(1.0)).max(MIN_KNOB)
    } else {
        track.width()
    }
    .min(track.width());
    let travel = (track.width() - knob_w).max(0.0);
    let knob_x = if max_left > 0.0 {
        track.left() + travel * (left / max_left).clamp(0.0, 1.0)
    } else {
        track.left()
    };
    let knob = Rect::from_min_size(
        egui::pos2(knob_x, track.top()),
        Vec2::new(knob_w, track.height()),
    );
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let hovered = response.hovered() || response.dragged();
    painter.rect_filled(
        knob,
        CornerRadius::same(4),
        if hovered { pal.muted } else { pal.knob },
    );
    if max_left <= 0.0 {
        return None;
    }
    let pointer = response.interact_pointer_pos()?;
    let grab_id = id.with("grab");
    if response.drag_started() || response.clicked() {
        if knob.contains(pointer) {
            ui.data_mut(|d| d.insert_temp(grab_id, pointer.x - knob.left()));
        } else {
            ui.data_mut(|d| d.remove::<f32>(grab_id));
            let next = if pointer.x < knob.left() {
                left - visible
            } else {
                left + visible
            };
            return Some(next.clamp(0.0, max_left));
        }
    }
    if response.dragged()
        && travel > 0.0
        && let Some(grab) = ui.data(|d| d.get_temp::<f32>(grab_id))
    {
        let x = pointer.x - grab - track.left();
        return Some((x / travel).clamp(0.0, 1.0) * max_left);
    }
    None
}
