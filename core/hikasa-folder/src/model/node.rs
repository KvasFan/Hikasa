//! Public node types of the model (§1.6.3): [`NodeId`], [`NodeKind`], [`NodeFlags`], [`ScanStatus`],
//! [`ContentSummary`], [`NodeView`], [`EntryView`], [`NodeError`]. Storage is private (`store.rs`); views keep
//! the API stable.

use hikasa_core::{ContentKind, ContentMethod, ErrorReport, FileTime, Side};
use hikasa_vfs::{Attrs, EntryKind};
use serde::{Deserialize, Serialize};

use crate::quick::DiffFlags;
use crate::state::CompareState;
use crate::time_cmp::TimeRelation;

/// Node id: arena index, unique within one comparison; `NodeId::ROOT` is the base-folder pair. Ids may differ
/// between runs that use parallelism; structure and states never do (compare with `to_canonical`). Ids are never
/// printed (JSON and reports use paths).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The base-folder pair.
    pub const ROOT: NodeId = NodeId(0);
}

/// Node kind (kinds never mix within a node, D-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A folder pair.
    Folder,
    /// Regular files (or followed links to files).
    File,
    /// Unfollowed symbolic links.
    Symlink,
    /// Anything else (devices, FIFOs, sockets; and entries whose kind could not be read).
    Other,
}

impl NodeKind {
    /// The node kind of an entry kind (`Dir` → `Folder`).
    pub const fn of(kind: EntryKind) -> NodeKind {
        match kind {
            EntryKind::Dir => NodeKind::Folder,
            EntryKind::File => NodeKind::File,
            EntryKind::Symlink => NodeKind::Symlink,
            EntryKind::Other => NodeKind::Other,
        }
    }

    /// The entry kind of the node's entries (`Folder` → `Dir`).
    pub const fn entry_kind(self) -> EntryKind {
        match self {
            NodeKind::Folder => EntryKind::Dir,
            NodeKind::File => EntryKind::File,
            NodeKind::Symlink => EntryKind::Symlink,
            NodeKind::Other => EntryKind::Other,
        }
    }

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            NodeKind::Folder => "folder",
            NodeKind::File => "file",
            NodeKind::Symlink => "symlink",
            NodeKind::Other => "other",
        }
    }
}

bitflags::bitflags! {
    /// Per-node facts that are not states.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct NodeFlags: u16 {
        /// Hidden by a filter on the left (FC-054).
        const LEFT_FILTERED = 1;
        /// Hidden by a filter on the right.
        const RIGHT_FILTERED = 2;
        /// The left entry is name-only (D-45).
        const LEFT_STAT_SKIPPED = 4;
        /// The right entry is name-only.
        const RIGHT_STAT_SKIPPED = 8;
        /// A per-entry error or a failed listing on the left (details: `TreeRead::errors`).
        const LEFT_ERROR = 16;
        /// The same on the right.
        const RIGHT_ERROR = 32;
        /// A followed symlink closed a loop (D-33).
        const SYMLINK_LOOP = 64;
        /// The rules-based method fell back to bytes because of the size limit (D-25).
        const RULES_SKIPPED_SIZE = 128;
        /// A content test was requested but skipped because the quick tests said "same" (FC-026).
        const CONTENT_SKIPPED = 256;
        /// An archive by its name, closed or opened, whichever side has it (FC-015, D-151).
        const ARCHIVE = 512;
        /// The left side of this folder is an opened archive: below it specs are members (D-151). Internal: not
        /// in the JSON names.
        const LEFT_ARCHIVE_ROOT = 1024;
        /// The same on the right.
        const RIGHT_ARCHIVE_ROOT = 2048;
    }
}

/// JSON names of the node flags in bit order (§4.1 rule 9).
const FLAG_NAMES: [(NodeFlags, &str); 10] = [
    (NodeFlags::LEFT_FILTERED, "left_filtered"),
    (NodeFlags::RIGHT_FILTERED, "right_filtered"),
    (NodeFlags::LEFT_STAT_SKIPPED, "left_stat_skipped"),
    (NodeFlags::RIGHT_STAT_SKIPPED, "right_stat_skipped"),
    (NodeFlags::LEFT_ERROR, "left_error"),
    (NodeFlags::RIGHT_ERROR, "right_error"),
    (NodeFlags::SYMLINK_LOOP, "symlink_loop"),
    (NodeFlags::RULES_SKIPPED_SIZE, "rules_skipped_size"),
    (NodeFlags::CONTENT_SKIPPED, "content_skipped"),
    (NodeFlags::ARCHIVE, "archive"),
];

impl NodeFlags {
    /// The set flags as snake_case names in bit order (`["left_filtered", …]`), the JSON form of bit sets
    /// (§4.1 rule 9). Additive API.
    pub fn names(self) -> Vec<&'static str> {
        FLAG_NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }

    /// `LEFT_FILTERED` or `RIGHT_FILTERED`.
    pub const fn filtered(side: Side) -> NodeFlags {
        match side {
            Side::Right => NodeFlags::RIGHT_FILTERED,
            Side::Left | Side::Center | Side::Output => NodeFlags::LEFT_FILTERED,
        }
    }

    /// `LEFT_ARCHIVE_ROOT` or `RIGHT_ARCHIVE_ROOT`.
    pub const fn archive_root(side: Side) -> NodeFlags {
        match side {
            Side::Right => NodeFlags::RIGHT_ARCHIVE_ROOT,
            Side::Left | Side::Center | Side::Output => NodeFlags::LEFT_ARCHIVE_ROOT,
        }
    }

    /// `LEFT_STAT_SKIPPED` or `RIGHT_STAT_SKIPPED`.
    pub const fn stat_skipped(side: Side) -> NodeFlags {
        match side {
            Side::Right => NodeFlags::RIGHT_STAT_SKIPPED,
            Side::Left | Side::Center | Side::Output => NodeFlags::LEFT_STAT_SKIPPED,
        }
    }

    /// `LEFT_ERROR` or `RIGHT_ERROR`.
    pub const fn error(side: Side) -> NodeFlags {
        match side {
            Side::Right => NodeFlags::RIGHT_ERROR,
            Side::Left | Side::Center | Side::Output => NodeFlags::LEFT_ERROR,
        }
    }
}

/// Scan status of a node (`NotApplicable` for file-like nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    /// A file-like node.
    NotApplicable,
    /// A folder that was not listed (pending, cancelled, excluded, or an orphan folder not scanned).
    NotScanned,
    /// A folder whose children exist.
    Scanned,
    /// A folder whose listing failed on a side (or that closed a symlink loop).
    Failed,
}

impl ScanStatus {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ScanStatus::NotApplicable => "not_applicable",
            ScanStatus::NotScanned => "not_scanned",
            ScanStatus::Scanned => "scanned",
            ScanStatus::Failed => "failed",
        }
    }
}

/// Compact content result of a node (the full outcome: `TreeRead::content_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentSummary {
    /// Method that ran (after fallbacks the method requested, e.g. `Rules` with `RULES_SKIPPED_SIZE`).
    pub method: ContentMethod,
    /// Result kind.
    pub kind: ContentKind,
}

/// A per-node error (failed listing, failed stat, failed content test, symlink loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    /// Side concerned (`None` = the pair).
    pub side: Option<Side>,
    /// The error.
    pub error: ErrorReport,
}

/// Borrowed view of one side's entry. Storage is private (§1.6.6); views keep the API stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryView<'a> {
    /// Name (UTF-8, lossy when `name_lossy`).
    pub name: &'a str,
    /// The OS name was not valid UTF-8.
    pub name_lossy: bool,
    /// Kind (of the target when `followed`). An entry whose metadata could not be read (side error flag, no
    /// metadata) reports its node's kind.
    pub kind: EntryKind,
    /// Bytes (files); 0 for folders and name-only entries.
    pub size: u64,
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Attrs,
    /// Symlink target text.
    pub link_target: Option<&'a str>,
    /// The entry is a followed symlink.
    pub followed: bool,
    /// Back-end or computed CRC32.
    pub crc32: Option<u32>,
    /// Hidden by a filter on this side (FC-054).
    pub filtered: bool,
    /// Name-only entry (D-45): size 0, no times, `Attrs::None`.
    pub stat_skipped: bool,
    /// Flatten mode (FC-014 Path column): the folder containing the entry, relative to the base ("" = the base);
    /// `None` outside flatten mode.
    pub dir: Option<&'a str>,
}

/// Borrowed view of one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeView<'a> {
    /// Id.
    pub id: NodeId,
    /// Parent (`None` for the root).
    pub parent: Option<NodeId>,
    /// Kind.
    pub kind: NodeKind,
    /// Canonical state (folders: derived from their counters, §2.6.4).
    pub state: CompareState,
    /// Detail flags (all differences found, including those cancelled by override, D-48).
    pub diffs: DiffFlags,
    /// Timestamp relation, when compared.
    pub time: Option<TimeRelation>,
    /// Content result, when a content test ran.
    pub content: Option<ContentSummary>,
    /// Scan status.
    pub scan: ScanStatus,
    /// Flags.
    pub flags: NodeFlags,
    /// Left entry.
    pub left: Option<EntryView<'a>>,
    /// Right entry.
    pub right: Option<EntryView<'a>>,
}

impl<'a> NodeView<'a> {
    /// The entry of `side` (`None` for `Center`/`Output`). Additive API.
    pub fn side(&self, side: Side) -> Option<&EntryView<'a>> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Right => self.right.as_ref(),
            Side::Center | Side::Output => None,
        }
    }

    /// The representative entry: left when present, else right. Additive API.
    pub fn representative(&self) -> Option<&EntryView<'a>> {
        self.left.as_ref().or(self.right.as_ref())
    }

    /// True for folder nodes. Additive API.
    pub fn is_folder(&self) -> bool {
        self.kind == NodeKind::Folder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_side_flags() {
        assert_eq!(
            (NodeFlags::SYMLINK_LOOP | NodeFlags::LEFT_FILTERED).names(),
            ["left_filtered", "symlink_loop"]
        );
        assert_eq!(NodeFlags::all().names().len(), 10);
        assert_eq!(
            NodeFlags::archive_root(Side::Right),
            NodeFlags::RIGHT_ARCHIVE_ROOT
        );
        assert_eq!(NodeFlags::filtered(Side::Right), NodeFlags::RIGHT_FILTERED);
        assert_eq!(
            NodeFlags::stat_skipped(Side::Left),
            NodeFlags::LEFT_STAT_SKIPPED
        );
        assert_eq!(NodeFlags::error(Side::Right), NodeFlags::RIGHT_ERROR);
        for kind in EntryKind::ALL {
            assert_eq!(NodeKind::of(kind).entry_kind(), kind);
        }
        assert_eq!(NodeKind::Folder.as_str(), "folder");
        assert_eq!(ScanStatus::NotScanned.as_str(), "not_scanned");
        assert_eq!(
            serde_json::to_string(&ScanStatus::NotApplicable).unwrap(),
            "\"not_applicable\""
        );
        assert_eq!(serde_json::to_string(&NodeId(7)).unwrap(), "7");
    }
}
