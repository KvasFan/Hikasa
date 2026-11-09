//! Display orders and cell values of the folder views (M2 W9a, D-235, D-237): the sort of the visible rows by a column
//! on one side, decided here so a window compares nothing (T11), and the pure texts every surface shares (the
//! extension of a name, the attribute text of an entry). Nothing here changes which rows exist or what an operation
//! reads: plans read the model in tree order.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;

use hikasa_core::{Pair, Side};
use serde::{Deserialize, Serialize};

use crate::align::canonical_fold;
use crate::counts::{Aggregates, SideTotals};
use crate::display::{DisplayOptions, VisibleRow, is_visible};
use crate::model::store::{Node, Store, internal};
use crate::model::{EntryView, NodeId, TreeRead};

/// A column a folder view sorts by (FC-040; the GUI's `ColumnId` without the fixed centre column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortColumn {
    /// The name (the D-20 key: case-folded NFC).
    Name,
    /// The extension (D-07: the text after the last `.` that is neither the first nor the last character).
    Ext,
    /// Bytes; a folder by the total below it on that side.
    Size,
    /// The modification time; none first.
    Modified,
    /// The attribute text ([`attributes_text`]).
    Attributes,
    /// The CRC32; unknown first.
    Crc,
    /// The numeric owner (uid).
    Owner,
    /// The numeric group (gid).
    Group,
    /// The folder below the base (flat comparisons), by the D-20 key per component.
    Path,
}

impl SortColumn {
    /// Every column in declaration order.
    pub const ALL: [SortColumn; 9] = [
        SortColumn::Name,
        SortColumn::Ext,
        SortColumn::Size,
        SortColumn::Modified,
        SortColumn::Attributes,
        SortColumn::Crc,
        SortColumn::Owner,
        SortColumn::Group,
        SortColumn::Path,
    ];

    /// The snake_case name (layout files, the serde form).
    pub const fn as_str(self) -> &'static str {
        match self {
            SortColumn::Name => "name",
            SortColumn::Ext => "ext",
            SortColumn::Size => "size",
            SortColumn::Modified => "modified",
            SortColumn::Attributes => "attributes",
            SortColumn::Crc => "crc",
            SortColumn::Owner => "owner",
            SortColumn::Group => "group",
            SortColumn::Path => "path",
        }
    }

    /// The column of a snake_case name.
    pub fn parse(text: &str) -> Option<SortColumn> {
        SortColumn::ALL.into_iter().find(|c| c.as_str() == text)
    }
}

/// A display order: a column on one side, ascending or descending (D-235).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SortKey {
    /// The column.
    pub column: SortColumn,
    /// The side whose values decide (`Left` or `Right`); a row without an item there is sorted by the other side's.
    pub side: Side,
    /// Largest first.
    pub descending: bool,
}

/// The D-07 extension of a name: the text after the last `.` when that `.` is neither the first nor the last
/// character (`.bashrc` and `a.` have none).
pub fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    name.get(dot + 1..)
}

/// The Attributes cell of an entry (FC-040): the DOS letters, or the Unix mode string; empty without attributes.
pub fn attributes_text(entry: &EntryView<'_>) -> String {
    if let Some(dos) = entry.attrs.dos() {
        dos.letters()
    } else if let Some(unix) = entry.attrs.unix() {
        unix.permissions_string(entry.kind)
    } else {
        String::new()
    }
}

/// One row's value of a column; `Missing` sorts first (ascending).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Value<'a> {
    Missing,
    Int(u64),
    Time(hikasa_core::FileTime),
    Text(Cow<'a, str>),
    Parts(Vec<Cow<'a, str>>),
}

/// The value of `column` for the raw node `node` (id `id`) on `side` (a row missing there by the other side's), read
/// from the store without building a `NodeView` (M2 W9a after review, G15 tree).
fn value_of<'a>(
    store: &'a Store,
    node: &Node,
    id: u32,
    column: SortColumn,
    side: Side,
) -> Value<'a> {
    let k = usize::from(side == Side::Right);
    let (from, entry) = match store.entry_view(node, id, k) {
        Some(entry) => (k, entry),
        None => match store.entry_view(node, id, 1 - k) {
            Some(entry) => (1 - k, entry),
            None => return Value::Missing,
        },
    };
    match column {
        SortColumn::Name => Value::Text(canonical_fold(entry.name)),
        SortColumn::Ext => extension(entry.name).map_or(Value::Missing, |ext| {
            Value::Text(Cow::Owned(canonical_fold(ext).into_owned()))
        }),
        SortColumn::Size => {
            if node.is_folder() {
                let totals = store.totals(node);
                Value::Int(if from == 1 {
                    totals.right.bytes
                } else {
                    totals.left.bytes
                })
            } else {
                Value::Int(entry.size)
            }
        }
        SortColumn::Modified => entry.mtime.map_or(Value::Missing, Value::Time),
        SortColumn::Attributes => Value::Text(Cow::Owned(attributes_text(&entry))),
        SortColumn::Crc => entry
            .crc32
            .map_or(Value::Missing, |c| Value::Int(u64::from(c))),
        SortColumn::Owner => entry
            .attrs
            .unix()
            .map_or(Value::Missing, |u| Value::Int(u64::from(u.uid))),
        SortColumn::Group => entry
            .attrs
            .unix()
            .map_or(Value::Missing, |u| Value::Int(u64::from(u.gid))),
        SortColumn::Path => match entry.dir {
            None | Some("") => Value::Parts(Vec::new()),
            Some(dir) => Value::Parts(dir.split('/').map(canonical_fold).collect()),
        },
    }
}

/// The ordering key of one sibling: folders first (D-20), the column's value, then the tree position.
struct Keyed<'a> {
    folder_rank: u8,
    kind: u8,
    value: Value<'a>,
    position: usize,
    id: u32,
}

/// What a sorted rebuild keeps for the next one (M2 W9a after review, G15 tree): each folder's visible children in the
/// sort's order with the stamp they were sorted at, so a rebuild sorts again only the folders that changed. A folder's
/// stamp is its children's range and its counters (categories and totals, which every change below it moves) while the
/// comparison runs; once it has ended, any change of the model sorts everything again (a refresh may move values and
/// keep the sums), as does a CRC found while the sort is on CRC. Another model, display or key starts it over.
#[derive(Debug, Default)]
pub struct SortCache {
    key: Option<(SortKey, DisplayOptions, usize)>,
    /// The model's generation once it had finished (`None` while it runs).
    finished_at: Option<u64>,
    cache_generation: u64,
    folders: HashMap<u32, (Stamp, Vec<u32>)>,
    /// The folders the last rebuild sorted (the others came from the cache; tests, the bench).
    pub sorted_last: usize,
}

/// A folder's children and counters when its order was cached.
type Stamp = (u32, u32, Aggregates, Pair<SideTotals>);

impl SortCache {
    /// Forgets every order.
    pub fn clear(&mut self) {
        *self = SortCache::default();
    }
}

impl TreeRead<'_> {
    /// [`TreeRead::visible_rows`] in the order of `sort` (D-235): inside each folder, folders before the other kinds
    /// (D-20), then by the key on its side (a row without an item there by the other side's value), ties by the
    /// tree's own order (D-20: case-folded NFC name). Only siblings move: every row stays under its parent, and the
    /// rows are exactly those of `visible_rows`. `None` is the tree order; `Name` ascending on the left is too.
    /// O(rows · log siblings).
    pub fn visible_rows_sorted(
        &self,
        opts: &DisplayOptions,
        expanded: &dyn Fn(NodeId) -> bool,
        sort: Option<&SortKey>,
        out: &mut Vec<VisibleRow>,
    ) {
        self.visible_rows_sorted_cached(opts, expanded, sort, &mut SortCache::default(), out);
    }

    /// [`TreeRead::visible_rows_sorted`] with the folders' orders kept in `cache` between rebuilds: only a folder
    /// whose children or counters changed is sorted again ([`SortCache`]). The rows are the same as without it.
    pub fn visible_rows_sorted_cached(
        &self,
        opts: &DisplayOptions,
        expanded: &dyn Fn(NodeId) -> bool,
        sort: Option<&SortKey>,
        cache: &mut SortCache,
        out: &mut Vec<VisibleRow>,
    ) {
        let Some(sort) = sort else {
            self.visible_rows(opts, expanded, out);
            return;
        };
        let model = self.model_ref();
        let key = (*sort, *opts, std::ptr::from_ref(model) as usize);
        if cache.key != Some(key) {
            cache.clear();
            cache.key = Some(key);
            cache.cache_generation = model.cache_generation();
        }
        if sort.column == SortColumn::Crc && cache.cache_generation != model.cache_generation() {
            cache.folders.clear();
            cache.cache_generation = model.cache_generation();
        }
        if model.is_finished() {
            let generation = model.generation();
            match cache.finished_at {
                // The scan's own changes are in the stamps; later ones (a refresh, a content test) sort all again.
                Some(at) if at != generation => cache.folders.clear(),
                _ => {}
            }
            cache.finished_at = Some(generation);
        } else {
            cache.finished_at = None;
        }
        cache.sorted_last = 0;
        let store = self.store();
        let mut stack: Vec<(u32, u32)> = Vec::new();
        let mut scratch: Vec<Keyed<'_>> = Vec::new();
        self.push_sorted(
            store,
            NodeId::ROOT.0,
            0,
            opts,
            sort,
            cache,
            &mut stack,
            &mut scratch,
        );
        while let Some((id, depth)) = stack.pop() {
            out.push(VisibleRow {
                node: NodeId(id),
                depth,
            });
            let open = store
                .node(NodeId(id))
                .is_some_and(|n| n.is_folder() && n.child_count > 0)
                && expanded(NodeId(id));
            if open {
                self.push_sorted(
                    store,
                    id,
                    depth + 1,
                    opts,
                    sort,
                    cache,
                    &mut stack,
                    &mut scratch,
                );
            }
        }
    }

    /// Pushes the visible children of `parent`, sorted, onto `stack` (last first, so they pop in order); the order
    /// comes from `cache` while the folder's stamp holds.
    #[allow(clippy::too_many_arguments)] // the walk's state
    fn push_sorted<'r>(
        &'r self,
        store: &'r Store,
        parent: u32,
        depth: u32,
        opts: &DisplayOptions,
        sort: &SortKey,
        cache: &mut SortCache,
        stack: &mut Vec<(u32, u32)>,
        scratch: &mut Vec<Keyed<'r>>,
    ) {
        let Some(node) = store.node(NodeId(parent)) else {
            return;
        };
        let stamp: Stamp = (
            node.first_child,
            node.child_count,
            store.aggregates(node),
            store.totals(node),
        );
        if let Some((at, order)) = cache.folders.get(&parent)
            && *at == stamp
        {
            stack.extend(order.iter().rev().map(|&id| (id, depth)));
            return;
        }
        cache.sorted_last += 1;
        scratch.clear();
        for (position, child) in node.children().enumerate() {
            let Some(raw) = store.node(NodeId(child)) else {
                continue;
            };
            if raw.internal & internal::GONE != 0 || !is_visible(store, raw, opts) {
                continue;
            }
            scratch.push(Keyed {
                folder_rank: u8::from(!raw.is_folder()),
                kind: raw.kind() as u8,
                value: value_of(store, raw, child, sort.column, sort.side),
                position,
                id: child,
            });
        }
        let name = sort.column == SortColumn::Name;
        scratch.sort_by(|a, b| {
            // Name keeps the tree's kind rank (Folder, File, Symlink, Other) so that Name ascending is the tree order;
            // the other columns put folders first and compare the value alone.
            let kind = |k: &Keyed<'_>| if name { k.kind } else { 0 };
            let by_value = (kind(a), &a.value).cmp(&(kind(b), &b.value));
            let by_value = if sort.descending {
                by_value.reverse()
            } else {
                by_value
            };
            a.folder_rank
                .cmp(&b.folder_rank)
                .then(by_value)
                .then(a.position.cmp(&b.position))
        });
        let order: Vec<u32> = scratch.iter().map(|k| k.id).collect();
        scratch.clear();
        stack.extend(order.iter().rev().map(|&id| (id, depth)));
        cache.folders.insert(parent, (stamp, order));
    }
}

/// Compares two rows by `sort` as [`TreeRead::visible_rows_sorted`] does for siblings, without the tree position
/// (tests and the bench: `Equal` means the tree's order decides).
pub fn compare_rows(read: &TreeRead<'_>, a: NodeId, b: NodeId, sort: &SortKey) -> Ordering {
    let store = read.store();
    let (Some(x), Some(y)) = (store.node(a), store.node(b)) else {
        return Ordering::Equal;
    };
    let rank = |n: &Node| u8::from(!n.is_folder());
    let name = sort.column == SortColumn::Name;
    let kind = |n: &Node| if name { n.kind() as u8 } else { 0 };
    let by_value = (kind(x), value_of(store, x, a.0, sort.column, sort.side))
        .cmp(&(kind(y), value_of(store, y, b.0, sort.column, sort.side)));
    rank(x).cmp(&rank(y)).then(if sort.descending {
        by_value.reverse()
    } else {
        by_value
    })
}

/// The direction of [`next_difference_file`] (TC-052: Next and Previous Difference Files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    /// Towards the end of the tree's pre-order (Next, Ctrl+M).
    Forward,
    /// Towards its start (Previous, Shift+Ctrl+M).
    Backward,
}

/// What [`next_difference_file`] looks at (OPT-007 and the parent view's display, contract §1.10.21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NextFileOptions {
    /// `[next_difference] limit_next_difference_files_to_current_folder`: only `from`'s siblings.
    pub limit_to_folder: bool,
    /// `[next_difference] wrap_around`: past the last candidate comes the first (and the other way round).
    pub wrap: bool,
    /// The parent folder view's display filter: a row it hides is never a target (folders count as all open).
    pub display: DisplayOptions,
    /// *Ignore unimportant differences*: a pair whose only differences are unimportant is no target.
    pub ignore_unimportant: bool,
}

/// The next (or previous) pair of files with differences after `from` (TC-052, D-288; contract §1.10.21).
///
/// Candidates are the rows with a **file on both sides** whose category is a difference (`different`,
/// `left_newer`, `right_newer`; under `opts.ignore_unimportant` a pair with only unimportant differences reads as
/// same), not marked Ignored (D-171), shown by `opts.display` with every folder open, taken in the tree's pre-order
/// over every scanned row: a closed folder of the view is looked into, as Find Filename does (D-241). Orphans,
/// folders, links and other kinds are never targets. With `opts.limit_to_folder` only `from`'s siblings are looked
/// at (the files of its folder, not below its sub-folders).
///
/// The search starts after `from` (before it, going `Backward`) and, with `opts.wrap`, goes on from the other end
/// until it comes back to `from`; `from` itself is never the answer. `from = NodeId::ROOT` starts at the first row
/// (the last one going backward). `None` when there is no candidate (the view's *no more differences* message), or
/// `from` is not a row of this model.
///
/// Pure, and walks only the rows it passes: no list of rows is built. O(rows passed + depth of `from`); a scan of a
/// whole tree of 100,000 rows takes about a millisecond.
pub fn next_difference_file(
    read: &TreeRead<'_>,
    from: NodeId,
    dir: Dir,
    opts: &NextFileOptions,
) -> Option<NodeId> {
    let store = read.store();
    store.node(from)?;
    let walk = Walk {
        store,
        display: &opts.display,
    };
    let candidate = |id: u32| walk.is_candidate(id, opts.ignore_unimportant);
    if opts.limit_to_folder {
        return walk.next_sibling_file(from.0, dir, opts.wrap, &candidate);
    }
    // A row inside a hidden folder is not shown: the walk starts at the highest hidden ancestor and never enters it.
    let start = walk.highest_hidden(from.0);
    walk.next_in_tree(start, dir, opts.wrap, &candidate)
}

impl TreeRead<'_> {
    /// [`next_difference_file`] on this guard (the Service table's `TreeRead::next_difference_file`).
    pub fn next_difference_file(
        &self,
        from: NodeId,
        dir: Dir,
        opts: &NextFileOptions,
    ) -> Option<NodeId> {
        next_difference_file(self, from, dir, opts)
    }
}

/// The pre-order walk of [`next_difference_file`]: siblings are a contiguous id block, so the next and previous rows
/// come from the parent's child range without any list.
struct Walk<'a> {
    store: &'a Store,
    display: &'a DisplayOptions,
}

impl Walk<'_> {
    /// The node `id` when it is a live row (not left behind by a refresh).
    fn live(&self, id: u32) -> Option<&Node> {
        self.store
            .node(NodeId(id))
            .filter(|node| node.internal & internal::GONE == 0)
    }

    /// True when the walk goes into `id`'s children: a shown folder.
    fn enters(&self, id: u32) -> bool {
        self.live(id).is_some_and(|node| {
            node.is_folder()
                && node.child_count > 0
                && (id == NodeId::ROOT.0 || is_visible(self.store, node, self.display))
        })
    }

    /// §1.10.21's candidate test of one row (its ancestors are the walk's business).
    fn is_candidate(&self, id: u32, ignore_unimportant: bool) -> bool {
        let Some(node) = self.live(id) else {
            return false;
        };
        node.kind() == crate::model::NodeKind::File
            && node.sides.iter().all(|side| side.present())
            && !node.flags().contains(crate::model::NodeFlags::IGNORED)
            && matches!(
                self.store.category_of(node, ignore_unimportant),
                crate::state::Category::Different
                    | crate::state::Category::LeftNewer
                    | crate::state::Category::RightNewer
            )
            && is_visible(self.store, node, self.display)
    }

    /// `id`, or its highest ancestor below the root that the display hides.
    fn highest_hidden(&self, id: u32) -> u32 {
        let mut highest = id;
        let mut current = id;
        while current != NodeId::ROOT.0 {
            let Some(node) = self.store.node(NodeId(current)) else {
                break;
            };
            if node.parent == crate::model::store::NONE {
                break;
            }
            let parent = node.parent;
            if parent != NodeId::ROOT.0 && !self.enters(parent) {
                highest = parent;
            }
            current = parent;
        }
        highest
    }

    /// The parent's child range of `id` (the root's own range for the root).
    fn siblings(&self, id: u32) -> Option<std::ops::Range<u32>> {
        let node = self.store.node(NodeId(id))?;
        if id == NodeId::ROOT.0 || node.parent == crate::model::store::NONE {
            return Some(node.children());
        }
        Some(self.store.node(NodeId(node.parent))?.children())
    }

    /// With `limit_to_folder`: the next candidate among `from`'s siblings, wrapping inside the folder. The folder must be
    /// shown (every ancestor entered).
    fn next_sibling_file(
        &self,
        from: u32,
        dir: Dir,
        wrap: bool,
        candidate: &dyn Fn(u32) -> bool,
    ) -> Option<NodeId> {
        let range = self.siblings(from)?;
        if range.is_empty() {
            return None;
        }
        // The folder itself and every folder above it must be shown.
        let mut folder = if from == NodeId::ROOT.0 {
            NodeId::ROOT.0
        } else {
            self.store.node(NodeId(from))?.parent
        };
        while folder != crate::model::store::NONE {
            if !self.enters(folder) {
                return None;
            }
            folder = self.store.node(NodeId(folder))?.parent;
        }
        let (start, end) = (range.start, range.end);
        let inside = range.contains(&from);
        let mut order: Box<dyn Iterator<Item = u32>> = match (dir, inside) {
            (Dir::Forward, true) => {
                let tail = from + 1..end;
                if wrap {
                    Box::new(tail.chain(start..from))
                } else {
                    Box::new(tail)
                }
            }
            (Dir::Backward, true) => {
                let head = (start..from).rev();
                if wrap {
                    Box::new(head.chain((from + 1..end).rev()))
                } else {
                    Box::new(head)
                }
            }
            // From the root: the root's children, from one end.
            (Dir::Forward, false) => Box::new(start..end),
            (Dir::Backward, false) => Box::new((start..end).rev()),
        };
        order.find(|&id| candidate(id)).map(NodeId)
    }

    /// The row after `id` in the shown pre-order (`enter`: `id`'s children come first when it is entered); `None` at the end.
    fn successor(&self, id: u32, enter: bool) -> Option<u32> {
        if enter && self.enters(id) {
            return self.live(id).map(|node| node.first_child);
        }
        let mut current = id;
        loop {
            if current == NodeId::ROOT.0 {
                return None;
            }
            let node = self.store.node(NodeId(current))?;
            let parent = node.parent;
            if parent == crate::model::store::NONE {
                return None;
            }
            let range = self.store.node(NodeId(parent))?.children();
            if current + 1 < range.end {
                return Some(current + 1);
            }
            current = parent;
        }
    }

    /// The last row of `id`'s shown subtree (`id` itself when it is not entered).
    fn last_below(&self, mut id: u32) -> u32 {
        while self.enters(id) {
            match self.live(id) {
                Some(node) => id = node.first_child + node.child_count - 1,
                None => break,
            }
        }
        id
    }

    /// The row before `id` in the shown pre-order; `None` before the first row.
    fn predecessor(&self, id: u32) -> Option<u32> {
        if id == NodeId::ROOT.0 {
            return None;
        }
        let node = self.store.node(NodeId(id))?;
        let parent = node.parent;
        if parent == crate::model::store::NONE {
            return None;
        }
        let range = self.store.node(NodeId(parent))?.children();
        if id > range.start {
            return Some(self.last_below(id - 1));
        }
        (parent != NodeId::ROOT.0).then_some(parent)
    }

    /// The first row of the shown pre-order, or the last one.
    fn first(&self, dir: Dir) -> Option<u32> {
        match dir {
            Dir::Forward => self.successor(NodeId::ROOT.0, true),
            Dir::Backward => {
                let last = self.last_below(NodeId::ROOT.0);
                (last != NodeId::ROOT.0).then_some(last)
            }
        }
    }

    /// The whole tree: the next candidate after `start` in `dir`, wrapping once with `wrap`; `start` is never one.
    fn next_in_tree(
        &self,
        start: u32,
        dir: Dir,
        wrap: bool,
        candidate: &dyn Fn(u32) -> bool,
    ) -> Option<NodeId> {
        let step = |id: u32| match dir {
            // A hidden start is not entered: its rows are not shown.
            Dir::Forward => self.successor(id, true),
            Dir::Backward => self.predecessor(id),
        };
        let from_root = start == NodeId::ROOT.0;
        let mut at = if from_root {
            self.first(dir)
        } else {
            match dir {
                Dir::Forward => self.successor(start, self.enters(start)),
                Dir::Backward => self.predecessor(start),
            }
        };
        let mut wrapped = from_root;
        loop {
            match at {
                Some(id) if id == start => return None,
                Some(id) => {
                    if candidate(id) {
                        return Some(NodeId(id));
                    }
                    at = step(id);
                }
                None => {
                    if !wrap || wrapped {
                        return None;
                    }
                    wrapped = true;
                    at = self.first(dir);
                }
            }
        }
    }
}
