//! [`CanonicalTree`]: the id-free canonical form of a comparison (T2 equality checks and golden files).

use hikasa_core::{ContentKind, ErrorKind, Side};
use hikasa_vfs::EntryKind;
use serde::Serialize;

use crate::model::TreeRead;
use crate::model::node::{NodeId, NodeKind, ScanStatus};
use crate::state::CompareState;
use crate::time_cmp::TimeRelation;

/// Id-free canonical tree: nested `(name, kind, state, diffs, content kind, flags, children)`; implements
/// `PartialEq` and `Serialize` for golden files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalTree {
    /// Whether the scan completed.
    pub complete: bool,
    /// The base-folder pair (its entries carry no name: base paths are host-specific) and everything below it.
    pub root: CanonicalNode,
}

/// One node of a [`CanonicalTree`]. Additive API (the contract leaves the layout open).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalNode {
    /// Representative name (left when present); `""` for the root.
    pub name: String,
    /// Kind.
    pub kind: NodeKind,
    /// Canonical state.
    pub state: CompareState,
    /// Scan status.
    pub scan: ScanStatus,
    /// Diff flag names (bit order).
    pub diffs: Vec<String>,
    /// Timestamp relation.
    pub time: Option<TimeRelation>,
    /// Content result kind.
    pub content: Option<ContentKind>,
    /// Node flag names (bit order).
    pub flags: Vec<String>,
    /// Errors (side and kind; messages are host-specific and left out).
    pub errors: Vec<CanonicalError>,
    /// Left entry.
    pub left: Option<CanonicalEntry>,
    /// Right entry.
    pub right: Option<CanonicalEntry>,
    /// Children in canonical order.
    pub children: Vec<CanonicalNode>,
}

/// One side of a [`CanonicalNode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalEntry {
    /// Name (UTF-8, lossy when needed).
    pub name: String,
    /// Kind.
    pub kind: EntryKind,
    /// Size (`EntryView::size`).
    pub size: u64,
    /// Flatten folder.
    pub dir: Option<String>,
}

/// One error of a [`CanonicalNode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalError {
    /// Side concerned (`None` = the pair).
    pub side: Option<Side>,
    /// Error kind.
    pub kind: ErrorKind,
}

/// Builds the canonical tree bottom-up (children always have larger ids than their parent), without
/// recursion.
pub(crate) fn build(tree: &TreeRead<'_>) -> CanonicalTree {
    let len = tree.len() as usize;
    let mut built: Vec<Option<CanonicalNode>> = Vec::with_capacity(len);
    built.resize_with(len, || None);
    for id in (0..len).rev() {
        let node_id = NodeId(u32::try_from(id).unwrap_or(u32::MAX));
        let Some(view) = tree.node(node_id) else {
            continue;
        };
        let children: Vec<CanonicalNode> = tree
            .children(node_id)
            .filter_map(|c| built.get_mut(c.0 as usize).and_then(Option::take))
            .collect();
        let root = node_id == NodeId::ROOT;
        let entry = |e: &crate::model::EntryView<'_>| CanonicalEntry {
            name: if root {
                String::new()
            } else {
                e.name.to_owned()
            },
            kind: e.kind,
            size: if root { 0 } else { e.size },
            dir: e.dir.map(str::to_owned),
        };
        let node = CanonicalNode {
            name: if root {
                String::new()
            } else {
                view.representative()
                    .map_or_else(String::new, |e| e.name.to_owned())
            },
            kind: view.kind,
            state: view.state,
            scan: view.scan,
            diffs: view.diffs.names().into_iter().map(str::to_owned).collect(),
            time: view.time,
            content: view.content.map(|c| c.kind),
            flags: view.flags.names().into_iter().map(str::to_owned).collect(),
            errors: tree
                .errors(node_id)
                .iter()
                .map(|e| CanonicalError {
                    side: e.side,
                    kind: e.error.kind,
                })
                .collect(),
            left: view.left.as_ref().map(entry),
            right: view.right.as_ref().map(entry),
            children,
        };
        if let Some(slot) = built.get_mut(id) {
            *slot = Some(node);
        }
    }
    let root = built
        .into_iter()
        .next()
        .flatten()
        .unwrap_or_else(|| CanonicalNode {
            name: String::new(),
            kind: NodeKind::Folder,
            state: CompareState::Unknown,
            scan: ScanStatus::NotScanned,
            diffs: Vec::new(),
            time: None,
            content: None,
            flags: Vec::new(),
            errors: Vec::new(),
            left: None,
            right: None,
            children: Vec::new(),
        });
    CanonicalTree {
        complete: tree.is_complete(),
        root,
    }
}
