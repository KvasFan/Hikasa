//! The help panel (M2 W0; UI-006, docs/17 §4.3): the topic the app core holds, drawn from the blocks of
//! `hikasa_session::help`. Links open other topics; Esc or the close button hides the panel.

use egui::{Button, RichText, ScrollArea, Ui};
use hikasa_session::help::{self, Block, Inline};

use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// What the user asked for while the panel was drawn.
pub enum HelpAction {
    /// Open another topic.
    Open(String),
    /// Hide the panel.
    Close,
}

/// Draws the topic `id` and returns the action the user took.
pub fn show(ui: &mut Ui, id: &str, pal: &Palette) -> Option<HelpAction> {
    let Some(topic) = help::topic(id) else {
        return Some(HelpAction::Close);
    };
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .add(Button::new("Contents").frame(false))
            .hint("The help contents")
            .clicked()
        {
            action = Some(HelpAction::Open(help::INDEX_TOPIC.to_owned()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(Button::new("×").frame(false))
                .hint("Close the help (Esc)")
                .clicked()
            {
                action = Some(HelpAction::Close);
            }
        });
    });
    ui.separator();
    ScrollArea::vertical()
        .id_salt(("hk-help", topic.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for block in topic.blocks() {
                match block {
                    Block::Heading { level, text } => {
                        let size = match level {
                            1 => 20.0,
                            2 => 16.0,
                            _ => 14.0,
                        };
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(help::plain(&text))
                                .size(size)
                                .strong()
                                .color(pal.fg),
                        );
                    }
                    Block::Paragraph(text) => {
                        inline(ui, &text, None, &mut action);
                    }
                    Block::Item(text) => {
                        inline(ui, &text, Some("•"), &mut action);
                    }
                    Block::Verbatim(lines) => {
                        ui.label(RichText::new(lines.trim_end()).monospace().size(12.0));
                    }
                }
            }
        });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(HelpAction::Close);
    }
    action
}

/// One paragraph or list item, wrapped, with clickable links.
fn inline(ui: &mut Ui, text: &[Inline], bullet: Option<&str>, action: &mut Option<HelpAction>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if let Some(bullet) = bullet {
            ui.label(format!("{bullet} "));
        }
        for part in text {
            match part {
                Inline::Text(t) => {
                    ui.label(t);
                }
                Inline::Code(t) => {
                    ui.code(t);
                }
                Inline::Link { label, topic } => {
                    if ui.link(label).clicked() {
                        *action = Some(HelpAction::Open(topic.clone()));
                    }
                }
            }
        }
    });
}
