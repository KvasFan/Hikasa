//! The Legend panel (M2 W9a, FC-030; D-242): a right-hand panel beside the folder tree, not a modal dialog, so it stays
//! open while the tree is used. It paints `legend::sections()` with the tree's own glyph painter and the current
//! palette, so it cannot disagree with the tree.

use egui::{Align, Align2, Color32, Layout, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};

use crate::folder::glyphs::{GlyphColour, IGNORED_BADGE};
use crate::legend::{LegendSection, Swatch};
use crate::ui::folder::draw_glyph;
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// What the user did in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendAction {
    /// Close (the `Legend` command toggles it off).
    Close,
    /// The panel's help topic.
    Help,
}

/// The size of a swatch.
const SWATCH: f32 = 18.0;

/// The colour a swatch paints in.
pub fn swatch_colour(swatch: Swatch, pal: &Palette) -> Color32 {
    match swatch {
        Swatch::Class(class, category) => pal.folder(class, category).unwrap_or(pal.f_text),
        Swatch::Glyph(glyph) => match glyph.colour {
            GlyphColour::Class => pal.f_text,
            GlyphColour::Unknown => pal.c_unknown,
            GlyphColour::Unopened => pal.c_unopened,
            GlyphColour::Error => pal.error,
        },
        Swatch::Mark(_) | Swatch::Ignored => pal.muted,
        Swatch::Marked => pal.accent,
    }
}

/// Draws the panel.
pub fn show(ui: &mut Ui, sections: &[LegendSection], pal: &Palette) -> Option<LegendAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Legend").strong().color(pal.fg));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("×").hint("Close (Ctrl+Alt+L)").clicked() {
                action = Some(LegendAction::Close);
            }
            if ui.small_button("?").hint("Help").clicked() {
                action = Some(LegendAction::Help);
            }
        });
    });
    ui.separator();
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for section in sections {
                ui.add_space(4.0);
                ui.label(RichText::new(section.title).strong().color(pal.fg));
                for entry in &section.entries {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::hover());
                        paint_swatch(ui, rect, entry.swatch, pal);
                        ui.add(egui::Label::new(RichText::new(&entry.text).color(pal.fg)).wrap());
                    });
                }
            }
        });
    action
}

/// Paints one swatch in `rect`.
fn paint_swatch(ui: &Ui, rect: egui::Rect, swatch: Swatch, pal: &Palette) {
    let painter = ui.painter();
    let colour = swatch_colour(swatch, pal);
    let at = rect.center();
    match swatch {
        Swatch::Class(..) => {
            painter.circle_filled(at, 4.5, colour);
        }
        Swatch::Glyph(glyph) => draw_glyph(painter, at, glyph, colour, pal),
        Swatch::Mark(mark) => {
            painter.text(
                at,
                Align2::CENTER_CENTER,
                mark,
                egui::FontId::proportional(13.0),
                colour,
            );
        }
        Swatch::Ignored => {
            painter.text(
                pos2(rect.right() - 2.0, rect.top() + 2.0),
                Align2::RIGHT_TOP,
                IGNORED_BADGE,
                egui::FontId::proportional(10.0),
                colour,
            );
        }
        Swatch::Marked => {
            painter.circle_stroke(at, SWATCH / 2.0 - 2.0, Stroke::new(1.5, colour));
        }
    }
}
