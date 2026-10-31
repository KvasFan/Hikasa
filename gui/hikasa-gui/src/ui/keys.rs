//! Keyboard events → core shortcuts (M2 W4a, D-111; UI-003): letters upper case, digits, `F1`…`F24`, the named
//! keys of the reference spelling and punctuation. On macOS a catalogue `Ctrl+` is the Command key, so egui's
//! `command` modifier counts as Ctrl. egui reports logical keys without telling the keypad apart, so `=`/`+`,
//! `-` and `0` pressed with Ctrl give two candidates ([`shortcuts_of`]): the key as typed (`Ctrl+=`, `Ctrl+-`,
//! `Ctrl+0`: a user binding of it wins) and then the keypad key of the font-size commands (`Ctrl+Num +`, …). A
//! key egui cannot report is simply not bindable.

use hikasa_session::settings::Shortcut;

/// The key name of an egui key in the reference spelling, or `None`.
fn key_name(key: egui::Key) -> Option<String> {
    use egui::Key as K;
    let named = match key {
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Escape => "Esc",
        K::Space => "Space",
        K::Backspace => "BkSp",
        K::Delete => "Del",
        K::Insert => "Ins",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PgUp",
        K::PageDown => "PgDn",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::Minus => "-",
        K::Plus => "+",
        other => {
            let name = other.symbol_or_name();
            let mut chars = name.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => {
                    Some(c.to_ascii_uppercase().to_string())
                }
                (Some(c), None) if c.is_ascii_punctuation() => Some(c.to_string()),
                (Some('F'), Some(_)) => name[1..]
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=24).contains(n))
                    .map(|n| format!("F{n}")),
                _ => None,
            };
        }
    };
    Some(named.to_owned())
}

/// The keypad key a Ctrl key may stand for (egui does not tell the keypad apart).
fn keypad_alias(key: egui::Key) -> Option<&'static str> {
    match key {
        egui::Key::Plus | egui::Key::Equals => Some("Num +"),
        egui::Key::Minus => Some("Num -"),
        egui::Key::Num0 => Some("Num 0"),
        _ => None,
    }
}

/// The core shortcuts of a key press, the first matching binding wins: the key as typed, then (Ctrl with `=`,
/// `+`, `-`, `0`) its keypad alias. Empty for a key the catalogue cannot bind.
pub fn shortcuts_of(key: egui::Key, modifiers: egui::Modifiers) -> Vec<Shortcut> {
    let ctrl = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
    let mut out = Vec::new();
    if let Some(name) = key_name(key) {
        out.push(Shortcut {
            shift: modifiers.shift,
            ctrl,
            alt: modifiers.alt,
            cmd: false,
            key: name,
            context: None,
        });
    }
    if ctrl && let Some(alias) = keypad_alias(key) {
        out.push(Shortcut {
            // Shift is part of typing `+` on most layouts.
            shift: modifiers.shift && alias != "Num +",
            ctrl,
            alt: modifiers.alt,
            cmd: false,
            key: alias.to_owned(),
            context: None,
        });
    }
    out
}

/// The first shortcut of a key press ([`shortcuts_of`]), or `None`.
#[cfg(test)]
pub fn shortcut_of(key: egui::Key, modifiers: egui::Modifiers) -> Option<Shortcut> {
    shortcuts_of(key, modifiers).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    fn sc(text: &str) -> Option<Shortcut> {
        Some(Shortcut::parse(text).expect("a shortcut"))
    }

    #[test]
    fn egui_keys_map_to_catalogue_shortcuts() {
        let ctrl = Modifiers::CTRL;
        assert_eq!(shortcut_of(egui::Key::T, ctrl), sc("Ctrl+T"));
        assert_eq!(
            shortcut_of(egui::Key::Tab, ctrl | Modifiers::SHIFT),
            sc("Shift+Ctrl+Tab")
        );
        assert_eq!(shortcut_of(egui::Key::F5, Modifiers::NONE), sc("F5"));
        assert_eq!(shortcut_of(egui::Key::F24, Modifiers::NONE), sc("F24"));
        assert_eq!(shortcut_of(egui::Key::F5, ctrl), sc("Ctrl+F5"));
        // The key as typed first (a user binding of `Ctrl+=` wins), then the keypad key.
        let all =
            |k, m| -> Vec<Option<Shortcut>> { shortcuts_of(k, m).into_iter().map(Some).collect() };
        assert_eq!(
            all(egui::Key::Plus, ctrl | Modifiers::SHIFT),
            [sc("Shift+Ctrl++"), sc("Ctrl+Num +")]
        );
        assert_eq!(
            all(egui::Key::Equals, ctrl),
            [sc("Ctrl+="), sc("Ctrl+Num +")]
        );
        assert_eq!(
            all(egui::Key::Minus, ctrl),
            [sc("Ctrl+-"), sc("Ctrl+Num -")]
        );
        assert_eq!(all(egui::Key::Num0, ctrl), [sc("Ctrl+0"), sc("Ctrl+Num 0")]);
        assert_eq!(all(egui::Key::Num0, Modifiers::NONE), [sc("0")]);
        assert_eq!(shortcut_of(egui::Key::Num0, Modifiers::NONE), sc("0"));
        assert_eq!(shortcut_of(egui::Key::Enter, Modifiers::NONE), sc("Enter"));
        assert_eq!(
            shortcut_of(egui::Key::ArrowRight, Modifiers::NONE),
            sc("Right")
        );
        assert_eq!(shortcut_of(egui::Key::Escape, Modifiers::NONE), sc("Esc"));
        assert_eq!(shortcut_of(egui::Key::Home, Modifiers::ALT), sc("Alt+Home"));
        assert_eq!(
            shortcut_of(egui::Key::Delete, Modifiers::SHIFT),
            sc("Shift+Del")
        );
        assert_eq!(shortcut_of(egui::Key::Minus, Modifiers::NONE), sc("-"));
        assert_eq!(shortcut_of(egui::Key::Slash, ctrl), sc("Ctrl+/"));
        // Cmd on macOS is Ctrl in the catalogue.
        assert_eq!(shortcut_of(egui::Key::W, Modifiers::MAC_CMD), sc("Ctrl+W"));
        // Every produced shortcut parses back to itself.
        for key in egui::Key::ALL {
            for m in [Modifiers::NONE, ctrl, Modifiers::ALT | Modifiers::SHIFT] {
                for s in shortcuts_of(*key, m) {
                    assert_eq!(Shortcut::parse(&s.to_string()).as_ref(), Ok(&s), "{key:?}");
                }
            }
        }
    }
}
