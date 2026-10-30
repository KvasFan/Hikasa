//! The Home view: two path fields and Compare (UI-001). The fields are passed verbatim to the model; the
//! model refuses what a window cannot open and resolves the paths on a job.

use egui::{Align, Button, FontId, Frame, Layout, Margin, RichText, TextEdit, Ui, vec2};

use hikasa_session::ipc::ViewId;

use crate::commands::CommandId;
use crate::home::HomeModel;
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// The id of a Home path field (its keyboard focus: the path-field shortcuts, D-111).
pub fn field_id(view: ViewId, right: bool) -> egui::Id {
    egui::Id::unique(("hikasa-home-field", view.0, right))
}

/// Draws the Home form; returns `Compare` when it was asked for (button or Enter in a field) and the rectangles
/// of the left and right path fields (drop targets, UI-008).
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    home: &mut HomeModel,
    pal: &Palette,
) -> (Option<CommandId>, [egui::Rect; 2]) {
    let mut run = None;
    let mut fields = [egui::Rect::NOTHING; 2];
    let ready = home.ready() && !home.resolving();
    let width = ui.available_width().min(640.0) - 32.0;
    ui.add_space((ui.available_height() / 2.0 - 150.0).max(16.0));
    ui.vertical_centered(|ui| {
        Frame::NONE
            .fill(pal.t_bg)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(32, 28))
            .show(ui, |ui| {
                ui.set_width(width);
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.label(RichText::new("Hikasa").size(22.0).strong().color(pal.t_text));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Compare two folders or two files. Paths may be relative to the folder Hikasa was \
                             started in.",
                        )
                        .color(pal.muted),
                    );
                    ui.add_space(10.0);
                    let mut enter = false;
                    for (caption, right) in [("Left", false), ("Right", true)] {
                        ui.horizontal(|ui| {
                            ui.add_sized(vec2(44.0, 24.0), egui::Label::new(RichText::new(caption).color(pal.muted)));
                            let value = if right { &mut home.right } else { &mut home.left };
                            let hint = if right { "/path/to/right" } else { "/path/to/left" };
                            let field = ui.add(
                                TextEdit::singleline(value)
                                    .id(field_id(view, right))
                                    .hint_text(hint)
                                    .font(FontId::monospace(13.0))
                                    .desired_width(f32::INFINITY),
                            );
                            fields[usize::from(right)] = field.rect;
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
                    let row = vec2(ui.available_width(), 30.0);
                    ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                        let compare = ui
                            .add_enabled(ready, Button::new(RichText::new("Compare").strong()).min_size(vec2(96.0, 28.0)))
                            .hint("Compares the two paths; the comparison replaces the Home view.");
                        if compare.clicked() || (enter && ready) {
                            run = Some(CommandId::HomeCompare);
                        }
                    });
                    if home.resolving() {
                        ui.label(RichText::new("Opening…").color(pal.muted));
                    }
                    if let Some(error) = &home.error {
                        ui.add_space(4.0);
                        Frame::NONE
                            .fill(pal.error_bg)
                            .inner_margin(Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(error).color(pal.error));
                            });
                    }
                });
            });
    });
    (run, fields)
}
