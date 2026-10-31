//! The live model (§1.6.4): [`FolderModel`] (created empty, filled by `compare_into`, readable at any time),
//! [`TreeRead`] (a short read guard with every query), the frozen [`FolderComparison`] and its id-free
//! [`CanonicalTree`].

mod canonical;
mod check;
pub(crate) mod node;
pub(crate) mod store;

use std::ffi::OsStr;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hikasa_core::{
    CaseSensitivity, CompareVerdict, ContentOutcome, InvariantViolation, Pair, RelPath, Side, Spec,
    Warning,
};
use hikasa_vfs::{Capabilities, Entry};
use parking_lot::{RwLock, RwLockReadGuard};

pub use canonical::{CanonicalEntry, CanonicalError, CanonicalNode, CanonicalTree};
pub use node::{
    ContentSummary, EntryView, NodeError, NodeFlags, NodeId, NodeKind, NodeView, ScanStatus,
};

use crate::counts::{Aggregates, SideTotals};
use crate::display::{ColourClass, DisplayOptions, VisibleRow, colour_of, visible_rows_into};
use crate::error::FolderError;
use crate::resolve::side_index;
use crate::rules::FolderRules;
use crate::state::Category;
use crate::stats::FolderStats;
use store::{NONE, Store};

/// One base folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseInfo {
    /// Spec.
    pub spec: Spec,
    /// Entry of the base folder.
    pub entry: Entry,
    /// Back-end capabilities.
    pub capabilities: Capabilities,
}

/// The comparison model: created empty, filled by `compare_into`, readable at any time (D-55). `Send + Sync`;
/// shared as `Arc<FolderModel>` by the scanner, the GUI and reports.
pub struct FolderModel {
    specs: Pair<Spec>,
    rules: FolderRules,
    store: RwLock<Store>,
    generation: AtomicU64,
    finished: AtomicBool,
    claimed: AtomicBool,
}

impl fmt::Debug for FolderModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FolderModel")
            .field("left", &self.specs.left.canonical())
            .field("right", &self.specs.right.canonical())
            .field("generation", &self.generation())
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

/// `name` below `spec`; with `opened`, the root of the archive that name is.
fn join_name(spec: Spec, name: &OsStr, opened: bool) -> Option<Spec> {
    let child = match spec {
        Spec::Local(path) => Spec::Local(path.join_name(name).ok()?),
        Spec::ArchiveMember { archive, member } => {
            let name = name.to_str()?;
            let joined = if member.is_empty() {
                name.to_owned()
            } else {
                format!("{member}/{name}")
            };
            Spec::archive_member(*archive, joined)
        }
        _ => return None,
    };
    Some(if opened {
        Spec::archive_member(child, "")
    } else {
        child
    })
}

impl FolderModel {
    /// Validates the rules (masks compile, ranges) and creates a model whose root is the base pair (state
    /// Unknown, `NotScanned`). No I/O.
    pub fn new(
        left: Spec,
        right: Spec,
        rules: FolderRules,
    ) -> Result<Arc<FolderModel>, FolderError> {
        rules.validate()?;
        let store = Store::new(
            rules.criteria.content.override_quick,
            rules.handling.flatten,
        );
        Ok(Arc::new(FolderModel {
            specs: Pair::new(left, right),
            rules,
            store: RwLock::new(store),
            generation: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
        }))
    }

    /// Short read access. The scanner holds the write lock for one directory batch or one content result at a
    /// time, never during I/O or event delivery; readers must hold the guard for one GUI frame or one report
    /// chunk at most.
    pub fn read(&self) -> TreeRead<'_> {
        TreeRead {
            store: self.store.read(),
            model: self,
        }
    }

    /// Incremented after every applied mutation (cheap change detection for the GUI).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// True once `compare_into` returned (complete or cancelled).
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// The two base specs.
    pub fn specs(&self) -> &Pair<Spec> {
        &self.specs
    }

    /// The rules.
    pub fn rules(&self) -> &FolderRules {
        &self.rules
    }

    /// Claims the model for one scan (`false` when it was claimed before).
    pub(crate) fn claim(&self) -> bool {
        !self.claimed.swap(true, Ordering::AcqRel)
    }

    /// Applies one mutation under the write lock and bumps the generation.
    pub(crate) fn mutate<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        let mut store = self.store.write();
        let result = f(&mut store);
        self.generation.fetch_add(1, Ordering::AcqRel);
        drop(store);
        result
    }

    /// Marks the model finished (after `compare_into`'s last mutation).
    pub(crate) fn set_finished(&self) {
        self.finished.store(true, Ordering::Release);
    }

    /// Bytes reserved by the node/folder storage and by the name arenas (bench `folder_model_1m`).
    #[doc(hidden)]
    pub fn reserved_bytes(&self) -> (usize, usize) {
        self.store.read().reserved_bytes()
    }
}

/// A read guard with every query. It hides the lock type (no parking_lot type in the API).
pub struct TreeRead<'a> {
    store: RwLockReadGuard<'a, Store>,
    model: &'a FolderModel,
}

impl fmt::Debug for TreeRead<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeRead")
            .field("len", &self.len())
            .field("complete", &self.is_complete())
            .finish_non_exhaustive()
    }
}

impl TreeRead<'_> {
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    /// `NodeId::ROOT`.
    pub fn root(&self) -> NodeId {
        NodeId::ROOT
    }

    /// Number of nodes so far.
    pub fn len(&self) -> u32 {
        self.store.len()
    }

    /// Always false: the root exists from the start. (Clippy's `len_without_is_empty` companion.)
    pub fn is_empty(&self) -> bool {
        self.store.len() == 0
    }

    /// A node (`None` for an id this model never issued).
    pub fn node(&self, id: NodeId) -> Option<NodeView<'_>> {
        self.store.view(id)
    }

    /// Children in canonical order (§2.5.4); a contiguous id block, empty for files and unlisted folders.
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        let range = self.store.node(id).map_or(0..0, |n| n.children());
        range.map(NodeId)
    }

    /// Relative path of a node (representative names, '/'-joined; the root is ""). In flatten mode a file's
    /// path is the representative side's folder plus its name.
    pub fn path(&self, id: NodeId) -> RelPath {
        let mut names: Vec<&str> = Vec::new();
        let mut current = id.0;
        let mut flat_dir: Option<&str> = None;
        while current != 0 && current != NONE {
            let Some(node) = self.store.node(NodeId(current)) else {
                break;
            };
            names.push(self.store.representative_name(node));
            if flat_dir.is_none() && current == id.0 {
                let side = if node.sides[0].present() { 0 } else { 1 };
                flat_dir = node.sides[side]
                    .dir()
                    .and_then(|d| self.store.dir_info(side, d))
                    .map(|d| d.text.as_str())
                    .filter(|d| !d.is_empty());
            }
            current = node.parent;
        }
        let mut text = String::new();
        if let Some(dir) = flat_dir {
            text.push_str(dir);
        }
        for name in names.iter().rev() {
            if !text.is_empty() {
                text.push('/');
            }
            text.push_str(name);
        }
        RelPath::parse(&text).unwrap_or_default()
    }

    /// Spec of a node's side, built from the base spec and the raw names; an opened archive on the way turns the
    /// names below it into members (D-151). `None` when the side is absent or the base is another kind of spec.
    /// The GUI opens file pairs with it (FC-062).
    pub fn spec(&self, id: NodeId, side: Side) -> Option<Spec> {
        if !matches!(side, Side::Left | Side::Right) {
            return None;
        }
        let s = side_index(side);
        let node = self.store.node(id)?;
        let rec = node.sides.get(s)?;
        if !rec.present() {
            return None;
        }
        if id == NodeId::ROOT {
            return Some(self.model.specs.get(side)?.clone());
        }
        let opened = NodeFlags::archive_root(side);
        // Flatten: the side's folder spec plus the name.
        if let Some(dir) = rec.dir() {
            let base = self.store.dir_info(s, dir)?.spec.clone()?;
            let raw = self.store.raw_name(s, id.0, rec);
            return join_name(base, &raw, node.flags().contains(opened));
        }
        let mut names = Vec::new();
        let mut current = id.0;
        while current != 0 {
            let node = self.store.node(NodeId(current))?;
            let rec = node.sides.get(s)?;
            if !rec.present() {
                return None;
            }
            names.push((
                self.store.raw_name(s, current, rec),
                node.flags().contains(opened),
            ));
            current = node.parent;
            if current == NONE {
                return None;
            }
        }
        let mut spec = self.model.specs.get(side)?.clone();
        for (name, opened) in names.iter().rev() {
            spec = join_name(spec, name, *opened)?;
        }
        Some(spec)
    }

    /// Display category (§2.6.3), O(1).
    pub fn category(&self, id: NodeId, ignore_unimportant: bool) -> Category {
        self.store.node(id).map_or(Category::Unknown, |n| {
            self.store.category_of(n, ignore_unimportant)
        })
    }

    /// Descendant counters of a folder (zero for file-like nodes).
    pub fn aggregates(&self, id: NodeId) -> Aggregates {
        self.store
            .node(id)
            .map(|n| self.store.aggregates(n))
            .unwrap_or_default()
    }

    /// Per-side totals of a subtree (a file: its own size).
    pub fn totals(&self, id: NodeId) -> Pair<SideTotals> {
        self.store
            .node(id)
            .map(|n| self.store.totals(n))
            .unwrap_or_default()
    }

    /// Colour class (FC-031 mapping, §2.6.4), O(1).
    pub fn colour(&self, id: NodeId, side: Side, ignore_unimportant: bool) -> ColourClass {
        match self.store.node(id) {
            Some(node) => colour_of(&self.store, node, side, ignore_unimportant),
            None => ColourClass::Absent,
        }
    }

    /// Errors of a node (empty when none).
    pub fn errors(&self, id: NodeId) -> &[NodeError] {
        self.store.errors(id)
    }

    /// The full content outcome (method, result with error report, warnings), when a content test ran.
    pub fn content_outcome(&self, id: NodeId) -> Option<ContentOutcome> {
        self.store.content_outcome(id)
    }

    /// Appends the visible rows in pre-order (§2.7); O(visible rows + children of expanded folders).
    /// `expanded` decides whether a visible folder's children are listed (CLI and reports: `|_| true`).
    pub fn visible_rows(
        &self,
        opts: &DisplayOptions,
        expanded: &dyn Fn(NodeId) -> bool,
        out: &mut Vec<VisibleRow>,
    ) {
        visible_rows_into(&self.store, opts, expanded, out);
    }

    /// Pre-order traversal (node, depth) over every node except the root; children in canonical order.
    pub fn preorder(&self) -> impl Iterator<Item = (NodeId, u32)> + '_ {
        let mut stack: Vec<(u32, u32)> = Vec::new();
        if let Some(root) = self.store.node(NodeId::ROOT) {
            stack.extend(root.children().rev().map(|c| (c, 0)));
        }
        std::iter::from_fn(move || {
            let (id, depth) = stack.pop()?;
            if let Some(node) = self.store.node(NodeId(id)) {
                stack.extend(node.children().rev().map(|c| (c, depth + 1)));
            }
            Some((NodeId(id), depth))
        })
    }

    /// Statistics (§2.6.5), O(1) from the root counters.
    pub fn stats(&self, ignore_unimportant: bool) -> FolderStats {
        let Some(root) = self.store.node(NodeId::ROOT) else {
            return FolderStats::default();
        };
        let agg = self.store.aggregates(root);
        let a = agg.get(ignore_unimportant);
        let totals = self.store.totals(root);
        FolderStats {
            files: a.files,
            folders: a.folders,
            left: totals.left,
            right: totals.right,
            content: self.store.content_counts,
        }
    }

    /// Overall verdict (§2.6.5, D-02), O(1).
    pub fn verdict(&self, ignore_unimportant: bool) -> CompareVerdict {
        let agg = self.aggregates(NodeId::ROOT);
        let d = |iu: bool| {
            let a = agg.get(iu);
            a.files.differences() + a.folders.differences()
        };
        let a = agg.get(ignore_unimportant);
        let problems = u64::from(a.files.unknown)
            + u64::from(a.files.error)
            + u64::from(a.folders.unknown)
            + u64::from(a.folders.error);
        let incomplete = !self.is_complete() || problems > 0;
        if d(true) > 0 {
            CompareVerdict::Differences
        } else if incomplete {
            CompareVerdict::Incomplete
        } else if !ignore_unimportant && d(false) > 0 {
            CompareVerdict::UnimportantOnly
        } else {
            CompareVerdict::NoDifferences
        }
    }

    /// Base folders once loaded (`Started` event).
    pub fn bases(&self) -> Pair<Option<&BaseInfo>> {
        Pair::new(
            self.store.bases.left.as_ref(),
            self.store.bases.right.as_ref(),
        )
    }

    /// Case sensitivity used for alignment and masks (after resolving `CaseMode::Auto`), once bases are loaded.
    pub fn case(&self) -> Option<CaseSensitivity> {
        self.store.case
    }

    /// True when the scan finished without cancellation.
    pub fn is_complete(&self) -> bool {
        self.store.complete
    }

    /// Comparison-level warnings (capability missing for a criterion, symlink loops, entry errors …). Sorted and
    /// deduplicated when the scan finishes.
    pub fn warnings(&self) -> &[Warning] {
        &self.store.warnings
    }

    /// The model's rules (a convenience for readers). Additive API.
    pub fn rules(&self) -> &FolderRules {
        &self.model.rules
    }

    /// Depth-first ids of `id`'s ancestors, nearest first (empty for the root). Additive API (consumers that
    /// mirror the tree re-read a node's ancestors after an event).
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut current = self.store.node(id).map_or(NONE, |n| n.parent);
        while current != NONE {
            out.push(NodeId(current));
            current = self.store.node(NodeId(current)).map_or(NONE, |n| n.parent);
        }
        out
    }
}

/// The finished comparison: the live model after `compare_into` returned. No writes happen any more, so reads
/// never wait. `Send + Sync`; cheap to clone (`Arc` inside). INV-FOLDER-1..10 (Appendix B).
#[derive(Clone)]
pub struct FolderComparison {
    model: Arc<FolderModel>,
}

impl fmt::Debug for FolderComparison {
    /// Specs, completeness and root statistics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let read = self.read();
        f.debug_struct("FolderComparison")
            .field("left", &self.model.specs.left.canonical())
            .field("right", &self.model.specs.right.canonical())
            .field("complete", &read.is_complete())
            .field("stats", &read.stats(false))
            .finish()
    }
}

impl FolderComparison {
    /// Wraps a finished model.
    pub(crate) fn new(model: Arc<FolderModel>) -> FolderComparison {
        FolderComparison { model }
    }

    /// The underlying model.
    pub fn model(&self) -> &Arc<FolderModel> {
        &self.model
    }

    /// Read access (all queries).
    pub fn read(&self) -> TreeRead<'_> {
        self.model.read()
    }

    /// Rules used.
    pub fn rules(&self) -> &FolderRules {
        &self.model.rules
    }

    /// False when cancelled (unscanned folders are `Unknown`).
    pub fn is_complete(&self) -> bool {
        self.read().is_complete()
    }

    /// Statistics.
    pub fn stats(&self, ignore_unimportant: bool) -> FolderStats {
        self.read().stats(ignore_unimportant)
    }

    /// Overall verdict (D-02).
    pub fn verdict(&self, ignore_unimportant: bool) -> CompareVerdict {
        self.read().verdict(ignore_unimportant)
    }

    /// Id-free canonical form for equality checks (T2) and golden files.
    pub fn to_canonical(&self) -> CanonicalTree {
        canonical::build(&self.read())
    }

    /// Re-checks INV-FOLDER-1..10 (recounts every counter).
    pub fn check_invariants(&self) -> Result<(), InvariantViolation> {
        check::check(&self.read())
    }
}

/// Checks INV-FOLDER-1..10 on a live model (also on a partial tree, between events). Additive API used by the
/// P-FOLDER-6 property ("counters equal a recount after every applied batch").
pub fn check_tree_invariants(tree: &TreeRead<'_>) -> Result<(), InvariantViolation> {
    check::check(tree)
}
