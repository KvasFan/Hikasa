//! Draws a window's notices (D-117) in its top-right corner, newest on top, with their buttons.

use egui::{Align, Align2, Frame, Layout, Margin, RichText, Ui};

use crate::notify::{Notice, NoticeAction, NoticeKind};
use crate::ui::hint::Hint as _;
use crate::ui::icons::{self, IconId};
use crate::ui::theme::Palette;

/// The icon of a notice kind.
pub fn icon_of(kind: NoticeKind) -> IconId {
    match kind {
        NoticeKind::Info => IconId::Info,
        NoticeKind::Warning => IconId::Warning,
        NoticeKind::Error => IconId::Error,
    }
}

/// Draws `notices` (newest first); returns the (key, action) pressed.
pub fn show(ui: &mut Ui, notices: &[&Notice], pal: &Palette) -> Option<(String, NoticeAction)> {
    if notices.is_empty() {
        return None;
    }
    let mut pressed = None;
    let ctx = ui.ctx().clone();
    let shown = egui::Area::new(egui::Id::unique("hk-notices"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_TOP, egui::vec2(-12.0, 64.0))
        .show(&ctx, |ui| {
            ui.set_max_width(420.0);
            for notice in notices {
                let (fill, colour) = match notice.kind {
                    NoticeKind::Info => (pal.chrome, pal.fg),
                    NoticeKind::Warning => (pal.warning_bg, pal.warning),
                    NoticeKind::Error => (pal.error_bg, pal.error),
                };
                Frame::NONE
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(400.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            icons::paint(ui.painter(), rect, icon_of(notice.kind), colour);
                            ui.add(
                                egui::Label::new(RichText::new(&notice.text).color(pal.fg)).wrap(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                                if ui.small_button("×").hint("Dismiss").clicked() {
                                    pressed = Some((notice.key.clone(), NoticeAction::Dismiss));
                                }
                            });
                        });
                        if !notice.actions.is_empty() {
                            ui.horizontal(|ui| {
                                for action in &notice.actions {
                                    if ui.button(action.label()).clicked() {
                                        pressed = Some((notice.key.clone(), *action));
                                    }
                                }
                            });
                        }
                    });
                ui.add_space(6.0);
            }
        });
    // An area that was on screen before a dialog opened stays under the modal, which takes its clicks; and a notice
    // of the Options dialog's files is answered with that dialog open (§1.4.7).
    ctx.move_to_top(shown.response.layer_id);
    pressed
}
