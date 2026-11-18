//! The hex view's Layout dialog (M2 W13a, contract §1.13.4, D-356), toolkit-free: bytes per line (1–256; presets 8,
//! 16, 24, 32, 64), group (1, 2, 4, 8), characters (ASCII, Latin-1, none), *Show addresses* and their radix, the
//! panes' layout, the context rows of the *Differences* filter (0–64), and *Use for new Hex Compare views*. Numbers
//! clamp in the widget (§1.4.13); the choices are core's enumerations. OK applies to the view (a new generation, the
//! top byte kept) as view state (`[view]`, never dirty); *Use for new…* also writes the Hex Compare defaults session.

use std::any::Any;

use hikasa_hex::{CharSet, HexLayout, Radix};
use hikasa_session::Service;
use hikasa_session::ipc::ViewId;

use super::PaneLayout;
use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};

/// The help topic of the dialog.
pub const LAYOUT_TOPIC: &str = "dialog-hex-layout";
/// The presets of bytes per line.
pub const BYTES_PRESETS: [u16; 5] = [8, 16, 24, 32, 64];
/// The groups.
pub const GROUPS: [u8; 4] = [1, 2, 4, 8];

/// The dialog's fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutChoice {
    /// The rows' layout.
    pub layout: HexLayout,
    /// *Show addresses*.
    pub addresses: bool,
    /// The panes' arrangement.
    pub panes: PaneLayout,
    /// Context rows of *Differences*.
    pub context: u8,
}

/// The Layout dialog.
pub struct LayoutDialog {
    /// The view.
    pub view: ViewId,
    /// The fields.
    pub choice: LayoutChoice,
    /// *Use for new Hex Compare views*.
    pub defaults: bool,
    chosen: Option<(LayoutChoice, bool)>,
}

impl LayoutDialog {
    /// The dialog over a view's layout.
    pub fn new(view: ViewId, choice: LayoutChoice) -> LayoutDialog {
        LayoutDialog {
            view,
            choice,
            defaults: false,
            chosen: None,
        }
    }

    /// Sets bytes per line, clamped to 1…256.
    pub fn set_bytes_per_line(&mut self, n: i64) {
        self.choice.layout.bytes_per_line = n.clamp(1, 256) as u16;
    }

    /// Sets the context rows, clamped to 0…64.
    pub fn set_context(&mut self, n: i64) {
        self.choice.context = n.clamp(0, 64) as u8;
    }

    /// Sets the group (one of [`GROUPS`]; another value is the nearest smaller one).
    pub fn set_group(&mut self, g: u8) {
        self.choice.layout.group = GROUPS.iter().rev().copied().find(|&x| x <= g).unwrap_or(1);
    }

    /// Sets the characters column.
    pub fn set_chars(&mut self, chars: CharSet) {
        self.choice.layout.chars = chars;
    }

    /// Sets the address radix.
    pub fn set_radix(&mut self, radix: Radix) {
        self.choice.layout.address = radix;
    }

    /// The choice after OK and whether it goes to the defaults session too (once).
    pub fn take_chosen(&mut self) -> Option<(LayoutChoice, bool)> {
        self.chosen.take()
    }
}

/// The `[view]` and `[display]` words of a layout choice (what *Use for new Hex Compare views* writes with
/// `session_apply_scope`, the keys `session edit --set` writes).
pub fn view_words(choice: &LayoutChoice) -> Vec<(String, String)> {
    let l = &choice.layout;
    vec![
        ("view.bytes_per_line".into(), l.bytes_per_line.to_string()),
        ("view.group".into(), l.group.to_string()),
        ("view.chars".into(), l.chars.as_str().to_owned()),
        ("view.address".into(), l.address.as_str().to_owned()),
        ("view.addresses".into(), choice.addresses.to_string()),
        ("view.layout".into(), choice.panes.word().to_owned()),
        ("view.context".into(), choice.context.to_string()),
    ]
}

impl DialogModel for LayoutDialog {
    fn title(&self) -> String {
        "Layout".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        LAYOUT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".into(),
                role: ButtonRole::Ok,
                enabled: true,
                default: true,
            },
            DialogButton {
                label: "Cancel".into(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role == ButtonRole::Ok {
            self.chosen = Some((self.choice, self.defaults));
        }
        DialogStep::Close
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
