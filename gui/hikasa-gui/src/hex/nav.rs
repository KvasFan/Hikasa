//! The hex view's Go To dialog (M2 W13a, contract §1.13.6, D-358), toolkit-free: the address text (MRU
//! `mru.hex_goto`), the default radix of bare digits (remembered in `state.dialogs."hex-goto"`) and the side. OK asks
//! the Service to parse the text (`Service::hex_address`, core's `parse_address`); a refused text keeps the dialog open
//! with the core's message. The view resolves the address against the caret and the file's length
//! (`AddressQuery::resolve`).

use std::any::Any;

use hikasa_core::Side;
use hikasa_hex::Radix;
use hikasa_hex::nav::AddressQuery;
use hikasa_session::Service;
use hikasa_session::ipc::ViewId;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};

/// The help topic of the dialog.
pub const GOTO_TOPIC: &str = "dialog-hex-go-to";
/// The dialog's name in `state.toml`'s `[dialogs]`.
pub const GOTO_DIALOG: &str = "hex-goto";

/// The Go To dialog.
pub struct HexGoToDialog {
    /// The view.
    pub view: ViewId,
    /// The address as typed.
    pub text: String,
    /// The radix of bare digits.
    pub radix: Radix,
    /// The side.
    pub side: Side,
    /// The recent addresses.
    pub recent: Vec<String>,
    /// The core's refusal of the text.
    pub error: Option<String>,
    chosen: Option<(Side, AddressQuery, String)>,
}

impl HexGoToDialog {
    /// The dialog for a view, on the focus side.
    pub fn new(view: ViewId, side: Side, radix: Radix, recent: Vec<String>) -> HexGoToDialog {
        HexGoToDialog {
            view,
            text: String::new(),
            radix,
            side,
            recent,
            error: None,
            chosen: None,
        }
    }

    /// The side, the parsed address and the text after OK (once).
    pub fn take_chosen(&mut self) -> Option<(Side, AddressQuery, String)> {
        self.chosen.take()
    }
}

impl DialogModel for HexGoToDialog {
    fn title(&self) -> String {
        "Go To".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        GOTO_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".into(),
                role: ButtonRole::Ok,
                enabled: !self.text.trim().is_empty(),
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

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match svc.hex_address(&self.text, self.radix) {
            Ok(query) => {
                self.chosen = Some((self.side, query, self.text.trim().to_owned()));
                DialogStep::Close
            }
            Err(e) => {
                self.error = Some(e.to_string());
                DialogStep::Stay
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
