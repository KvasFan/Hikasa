//! Hints of every widget (UI-007, OPT-017, OPT-019): `response.hint(text)` instead of egui's `on_hover_text`,
//! so each hint follows `show_hints`, `hint_show_ms` and `hint_hide_ms` ([`HintTiming`]). The window stores
//! the timing of its options in the context once per frame ([`set_timing`]); the hover state lives beside it.

use egui::{Context, Id, Response, WidgetText};

use crate::commands::HintTiming;
use crate::ui::menus::Hover;

fn timing_id() -> Id {
    Id::unique("hikasa-hint-timing")
}

fn hover_id() -> Id {
    Id::unique("hikasa-hint-hover")
}

/// Stores the hint timing of the options for this frame.
pub fn set_timing(ctx: &Context, timing: HintTiming) {
    ctx.data_mut(|d| d.insert_temp(timing_id(), timing));
}

/// A hint that honours the hint timing.
pub trait Hint {
    /// Shows `text` as the hint of this widget per the stored [`HintTiming`].
    fn hint(self, text: impl Into<WidgetText>) -> Response;
    /// Like [`Hint::hint`], at the pointer (a hint per row of a large widget).
    fn hint_at_pointer(self, text: impl Into<WidgetText>) -> Response;
}

fn with_hover(response: Response, show: impl FnOnce(Response) -> Response) -> Response {
    let ctx = response.ctx.clone();
    let timing: HintTiming = ctx.data(|d| d.get_temp(timing_id())).unwrap_or_default();
    let mut hover: Hover = ctx.data(|d| d.get_temp(hover_id())).unwrap_or_default();
    let response = hover.hint_with(response, &timing, show);
    ctx.data_mut(|d| d.insert_temp(hover_id(), hover));
    response
}

impl Hint for Response {
    fn hint(self, text: impl Into<WidgetText>) -> Response {
        with_hover(self, |r| r.on_hover_text(text))
    }

    fn hint_at_pointer(self, text: impl Into<WidgetText>) -> Response {
        with_hover(self, |r| r.on_hover_text_at_pointer(text))
    }
}
