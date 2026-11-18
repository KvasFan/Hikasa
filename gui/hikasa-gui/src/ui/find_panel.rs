//! The Find Filename panel (M2 W9a, FC-061; D-241): one line above the folder tree, not a dialog. The text field
//! (with the recent texts), the options, the sides, Find Next and Previous, the help and the close button; a bad
//! regular expression shows in the panel. It edits the view's `FindPanel`; the searches are commands.

use egui::{ComboBox, Key, RichText, TextEdit, Ui};
use hikasa_folder::select::SideSet;

use crate::find_panel::FindPanel;
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// What the user did in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindAction {
    /// Find Next (Enter, the button).
    Next,
    /// Find Previous.
    Prev,
    /// Close (Esc, ×).
    Close,
    /// The panel's help topic.
    Help,
}

/// Draws the panel.
pub fn show(ui: &mut Ui, panel: &mut FindPanel, pal: &Palette) -> Option<FindAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Find:").color(pal.fg));
        let edit = ui.add(
            TextEdit::singleline(&mut panel.text)
                .desired_width(220.0)
                .hint_text("Filename"),
        );
        if std::mem::take(&mut panel.focus) {
            edit.request_focus();
        }
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            action = Some(FindAction::Next);
        }
        if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            action = Some(FindAction::Close);
        }
        if !panel.recent.is_empty() {
            ComboBox::from_id_salt("hk-find-recent")
                .selected_text("")
                .width(18.0)
                .show_ui(ui, |ui| {
                    for text in panel.recent.clone() {
                        if ui.selectable_label(false, &text).clicked() {
                            panel.text = text;
                        }
                    }
                });
        }
        if ui.button("Next").hint("Find Next (F3)").clicked() {
            action = Some(FindAction::Next);
        }
        if ui
            .button("Previous")
            .hint("Find Previous (Shift+F3)")
            .clicked()
        {
            action = Some(FindAction::Prev);
        }
        ui.checkbox(&mut panel.case, "Match case");
        ui.checkbox(&mut panel.whole_word, "Whole word");
        ui.checkbox(&mut panel.regex, "Regular expression");
        ui.checkbox(&mut panel.wrap, "Wrap around");
        ui.checkbox(&mut panel.up, "Search up");
        let label = |s: SideSet| match s {
            SideSet::LEFT => "Left",
            SideSet::RIGHT => "Right",
            _ => "Both sides",
        };
        ComboBox::from_id_salt("hk-find-sides")
            .selected_text(label(panel.sides))
            .show_ui(ui, |ui| {
                for s in [SideSet::all(), SideSet::LEFT, SideSet::RIGHT] {
                    ui.selectable_value(&mut panel.sides, s, label(s));
                }
            });
        if ui.small_button("?").hint("Help").clicked() {
            action = Some(FindAction::Help);
        }
        if ui.small_button("×").hint("Close (Esc)").clicked() {
            action = Some(FindAction::Close);
        }
        if let Some(error) = &panel.error {
            ui.label(RichText::new(error).color(pal.error));
        }
    });
    action
}
