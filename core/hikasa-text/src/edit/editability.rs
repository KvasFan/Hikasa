//! Editability (TC-003, TC-004, FF-031; D-265, contract §1.10.7): why a pane may not be edited.

use serde::Serialize;

/// One reason a pane may not be edited, in the order of precedence (the first one is the reported one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditBlock {
    /// The side could not be loaded.
    LoadFailed,
    /// The comparison was cancelled (a later run clears it).
    Cancelled,
    /// The location cannot be written (read-only file system, no write access, media).
    ReadOnlyLocation,
    /// A member of an archive that cannot be written (a single-stream compressor, or a read-only level).
    ArchiveReadOnly,
    /// A conversion without a save path (*Sorted*, *External* without `save`).
    Converted,
    /// A document cut at the D-43 limits.
    Truncated,
    /// The parent folder session's side is read-only.
    ParentReadOnly,
    /// The session's `[specs] read_only_*`.
    SessionReadOnly,
    /// `--ro`, `--ro1`, `--ro2`.
    CommandLine,
    /// Binary content (lifted by the user's *edit anyway*, [`crate::edit::EditableDocument::allow_binary_edits`]).
    Binary,
}

impl EditBlock {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            EditBlock::LoadFailed => "load_failed",
            EditBlock::Cancelled => "cancelled",
            EditBlock::ReadOnlyLocation => "read_only_location",
            EditBlock::ArchiveReadOnly => "archive_read_only",
            EditBlock::Converted => "converted",
            EditBlock::Truncated => "truncated",
            EditBlock::ParentReadOnly => "parent_read_only",
            EditBlock::SessionReadOnly => "session_read_only",
            EditBlock::CommandLine => "command_line",
            EditBlock::Binary => "binary",
        }
    }

    /// The status text of §1.10.7's table: *Read-only* for a property of the source, *Editing disabled* for a
    /// choice of the session or the user, and nothing for a side without a document.
    pub const fn status_text(self) -> &'static str {
        match self {
            EditBlock::LoadFailed => "",
            EditBlock::ReadOnlyLocation
            | EditBlock::ArchiveReadOnly
            | EditBlock::Converted
            | EditBlock::Truncated => "Read-only",
            EditBlock::Cancelled
            | EditBlock::ParentReadOnly
            | EditBlock::SessionReadOnly
            | EditBlock::CommandLine
            | EditBlock::Binary => "Editing disabled",
        }
    }
}

/// Whether a pane may be edited, and every cause when it may not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Editability {
    /// Every cause, in [`EditBlock`]'s order, without duplicates.
    pub blocks: Vec<EditBlock>,
    /// False for a document without a location (`-`, `clipboard://`, text pairs): Save becomes Save As.
    pub can_save_in_place: bool,
}

impl Default for Editability {
    /// Editable, saved in place.
    fn default() -> Editability {
        Editability::EDITABLE
    }
}

impl Editability {
    /// Editable, saved in place.
    pub const EDITABLE: Editability = Editability {
        blocks: Vec::new(),
        can_save_in_place: true,
    };

    /// Editability with the given causes (sorted and deduplicated).
    pub fn with_blocks(
        blocks: impl IntoIterator<Item = EditBlock>,
        can_save_in_place: bool,
    ) -> Editability {
        let mut blocks: Vec<EditBlock> = blocks.into_iter().collect();
        blocks.sort();
        blocks.dedup();
        Editability {
            blocks,
            can_save_in_place,
        }
    }

    /// Adds a cause (kept sorted, without duplicates).
    pub fn add(&mut self, block: EditBlock) {
        if let Err(at) = self.blocks.binary_search(&block) {
            self.blocks.insert(at, block);
        }
    }

    /// True when no cause blocks editing.
    pub fn editable(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The first cause.
    pub fn first(&self) -> Option<EditBlock> {
        self.blocks.first().copied()
    }

    /// The File Info header's text: the first cause's (`""` when editable).
    pub fn status_text(&self) -> &'static str {
        self.first().map_or("", EditBlock::status_text)
    }
}
