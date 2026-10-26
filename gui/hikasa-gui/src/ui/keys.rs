//! Keyboard events → catalogue shortcuts (UI-003). egui reports logical keys without telling the keypad apart,
//! so `+`/`=`, `-` and `0` pressed with Ctrl stand for the keypad keys of the font-size commands.

use crate::commands::{Key, Shortcut};

/// The catalogue shortcut of a key press (letters and digits upper-cased), or `None` for a key the catalogue
/// cannot bind. Cmd (macOS) counts as Ctrl.
pub fn shortcut_of(key: egui::Key, modifiers: egui::Modifiers) -> Option<Shortcut> {
    let ctrl = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
    let key = match key {
        egui::Key::Enter => Key::Enter,
        egui::Key::Tab => Key::Tab,
        egui::Key::ArrowUp => Key::Up,
        egui::Key::ArrowDown => Key::Down,
        egui::Key::ArrowLeft => Key::Left,
        egui::Key::ArrowRight => Key::Right,
        egui::Key::Plus | egui::Key::Equals if ctrl => Key::NumPlus,
        egui::Key::Minus if ctrl => Key::NumMinus,
        egui::Key::Num0 if ctrl => Key::NumZero,
        other => {
            let name = other.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => Key::Char(c.to_ascii_uppercase()),
                (Some('F'), Some(_)) => {
                    Key::F(name[1..].parse().ok().filter(|n| (1..=12).contains(n))?)
                }
                _ => return None,
            }
        }
    };
    Some(Shortcut {
        ctrl,
        // Shift is part of typing `+` on most layouts.
        shift: modifiers.shift && key != Key::NumPlus,
        alt: modifiers.alt,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    #[test]
    fn egui_keys_map_to_catalogue_shortcuts() {
        let ctrl = Modifiers::CTRL;
        assert_eq!(
            shortcut_of(egui::Key::T, ctrl),
            Some(Shortcut::ctrl(Key::Char('T')))
        );
        assert_eq!(
            shortcut_of(egui::Key::Tab, ctrl | Modifiers::SHIFT),
            Some(Shortcut::shift_ctrl(Key::Tab))
        );
        assert_eq!(
            shortcut_of(egui::Key::F5, Modifiers::NONE),
            Some(Shortcut::plain(Key::F(5)))
        );
        assert_eq!(
            shortcut_of(egui::Key::F5, ctrl),
            Some(Shortcut::ctrl(Key::F(5)))
        );
        assert_eq!(
            shortcut_of(egui::Key::Plus, ctrl | Modifiers::SHIFT),
            Some(Shortcut::ctrl(Key::NumPlus))
        );
        assert_eq!(
            shortcut_of(egui::Key::Equals, ctrl),
            Some(Shortcut::ctrl(Key::NumPlus))
        );
        assert_eq!(
            shortcut_of(egui::Key::Minus, ctrl),
            Some(Shortcut::ctrl(Key::NumMinus))
        );
        assert_eq!(
            shortcut_of(egui::Key::Num0, ctrl),
            Some(Shortcut::ctrl(Key::NumZero))
        );
        assert_eq!(
            shortcut_of(egui::Key::Num0, Modifiers::NONE),
            Some(Shortcut::plain(Key::Char('0')))
        );
        assert_eq!(
            shortcut_of(egui::Key::Enter, Modifiers::NONE),
            Some(Shortcut::plain(Key::Enter))
        );
        assert_eq!(
            shortcut_of(egui::Key::ArrowRight, Modifiers::NONE),
            Some(Shortcut::plain(Key::Right))
        );
        assert_eq!(shortcut_of(egui::Key::Escape, Modifiers::NONE), None);
        assert_eq!(shortcut_of(egui::Key::Minus, Modifiers::NONE), None);
        // Cmd on macOS is Ctrl in the catalogue.
        assert_eq!(
            shortcut_of(egui::Key::W, Modifiers::MAC_CMD),
            Some(Shortcut::ctrl(Key::Char('W')))
        );
    }
}
