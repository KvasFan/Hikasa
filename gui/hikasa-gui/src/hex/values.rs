//! The hex view's value panel (M2 W13a, contract §1.13.8, D-360), toolkit-free: the submenus of *Little Endian Values*
//! and *Big Endian Values* (*Show*, *Integers in Decimal*, *Integers in Hexadecimal*) and the panel's help topic. The
//! lines are core's (`values::format_with` of `values::at` at the caret's byte), computed by the view.

use hikasa_hex::Radix;
use hikasa_session::api::HexValues;

/// The argument of Copy from the value panel's context menu (its lines, not the bytes).
pub const VALUES_COPY: &str = "hex-values";

/// The help topic of the panel.
pub const VALUES_TOPIC: &str = "panel-hex-values";

/// An item of a values submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuesChoice {
    /// *Show* of the little (`true`) or big endian block.
    Show(bool),
    /// The integers' radix.
    Radix(Radix),
}

impl ValuesChoice {
    /// The caption.
    pub fn caption(self) -> &'static str {
        match self {
            ValuesChoice::Show(_) => "Show",
            ValuesChoice::Radix(Radix::Decimal) => "Integers in Decimal",
            ValuesChoice::Radix(Radix::Hex) => "Integers in Hexadecimal",
        }
    }

    /// The submenu of the little (`true`) or big endian block.
    pub fn items(little: bool) -> [ValuesChoice; 3] {
        [
            ValuesChoice::Show(little),
            ValuesChoice::Radix(Radix::Decimal),
            ValuesChoice::Radix(Radix::Hex),
        ]
    }

    /// The item is checked for `values` and `radix`.
    pub fn checked(self, values: HexValues, radix: Radix) -> bool {
        match self {
            ValuesChoice::Show(true) => values.little(),
            ValuesChoice::Show(false) => values.big(),
            ValuesChoice::Radix(r) => r == radix,
        }
    }
}
