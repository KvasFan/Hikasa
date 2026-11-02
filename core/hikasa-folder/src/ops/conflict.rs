//! What an item of a plan needs asked (§1.6.11, D-178): [`Conflicts`], from the row's `EntryView`.

use hikasa_vfs::{Attrs, DosAttrs};
use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// Why an item needs a question (§1.6.11).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct Conflicts: u8 {
        /// It replaces or removes a read-only target.
        const READ_ONLY = 1;
        /// It replaces or removes a system file.
        const SYSTEM = 2;
        /// It replaces a target strictly newer than its source.
        const OVERWRITE_NEWER = 4;
        /// A `move` that replaces.
        const REPLACE_DURING_MOVE = 8;
        /// A removal with the bin whose spec has none.
        const NO_RECYCLE_BIN = 16;
    }
}

/// JSON names of the conflicts, in bit order.
const NAMES: [(Conflicts, &str); 5] = [
    (Conflicts::READ_ONLY, "read_only"),
    (Conflicts::SYSTEM, "system"),
    (Conflicts::OVERWRITE_NEWER, "overwrite_newer"),
    (Conflicts::REPLACE_DURING_MOVE, "replace_during_move"),
    (Conflicts::NO_RECYCLE_BIN, "no_recycle_bin"),
];

impl Conflicts {
    /// The set flags as snake_case names, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }

    /// What the attributes of a target that is replaced or removed say: DOS `R`, or a Unix mode without any write bit
    /// (the owner is not resolved, T1); DOS `S`.
    pub fn of_attrs(attrs: &Attrs) -> Conflicts {
        match attrs {
            Attrs::Dos { attrs } => {
                let mut found = Conflicts::empty();
                found.set(Conflicts::READ_ONLY, attrs.contains(DosAttrs::READONLY));
                found.set(Conflicts::SYSTEM, attrs.contains(DosAttrs::SYSTEM));
                found
            }
            Attrs::Unix { meta } if meta.mode & 0o222 == 0 => Conflicts::READ_ONLY,
            _ => Conflicts::empty(),
        }
    }
}
