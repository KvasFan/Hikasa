//! Display orders and cell values of the folder views (M2 W9a, D-235, D-237): the sort of the visible rows by a column
//! on one side, decided here so a window compares nothing (T11), and the pure texts every surface shares (the
//! extension of a name, the attribute text of an entry). Nothing here changes which rows exist or what an operation
//! reads: plans read the model in tree order.

use std::borrow::Cow;
use std::cmp::Ordering;

use hikasa_core::Side;
use serde::{Deserialize, Serialize};

use crate::align::canonical_fold;
use crate::display::{DisplayOptions, VisibleRow, is_visible};
use crate::model::{EntryView, NodeId, NodeView, TreeRead};

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

/// The entry of `side`, else the other side's (a row missing on the sorted side sorts by the other side).
fn entry_on<'n, 'a>(node: &'n NodeView<'a>, side: Side) -> Option<&'n EntryView<'a>> {
    node.side(side).or_else(|| node.side(side.opposite()))
}

/// The value of `column` for `node` on `side`.
fn value_of<'a>(
    read: &TreeRead<'_>,
    node: &NodeView<'a>,
    column: SortColumn,
    side: Side,
) -> Value<'a> {
    let Some(entry) = entry_on(node, side) else {
        return Value::Missing;
    };
    // The side the value was read from (the other side for a row missing on `side`).
    let from = if node.side(side).is_some() {
        side
    } else {
        side.opposite()
    };
    match column {
        SortColumn::Name => Value::Text(canonical_fold(entry.name)),
        SortColumn::Ext => extension(entry.name).map_or(Value::Missing, |ext| {
            Value::Text(Cow::Owned(canonical_fold(ext).into_owned()))
        }),
        SortColumn::Size => {
            if node.is_folder() {
                let totals = read.totals(node.id);
                Value::Int(match from {
                    Side::Right => totals.right.bytes,
                    _ => totals.left.bytes,
                })
            } else {
                Value::Int(entry.size)
            }
        }
        SortColumn::Modified => entry.mtime.map_or(Value::Missing, Value::Time),
        SortColumn::Attributes => Value::Text(Cow::Owned(attributes_text(entry))),
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
    kind: crate::model::NodeKind,
    value: Value<'a>,
    position: usize,
    id: NodeId,
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
        let Some(sort) = sort else {
            self.visible_rows(opts, expanded, out);
            return;
        };
        let mut stack: Vec<(NodeId, u32)> = Vec::new();
        let mut scratch: Vec<Keyed<'_>> = Vec::new();
        self.push_sorted(NodeId::ROOT, 0, opts, sort, &mut stack, &mut scratch);
        while let Some((id, depth)) = stack.pop() {
            out.push(VisibleRow { node: id, depth });
            let open = self
                .node(id)
                .is_some_and(|n| n.is_folder() && self.children(id).len() > 0)
                && expanded(id);
            if open {
                self.push_sorted(id, depth + 1, opts, sort, &mut stack, &mut scratch);
            }
        }
    }

    /// Pushes the visible children of `parent`, sorted, onto `stack` (last first, so they pop in order).
    fn push_sorted<'r>(
        &'r self,
        parent: NodeId,
        depth: u32,
        opts: &DisplayOptions,
        sort: &SortKey,
        stack: &mut Vec<(NodeId, u32)>,
        scratch: &mut Vec<Keyed<'r>>,
    ) {
        scratch.clear();
        let store = self.store();
        for (position, child) in self.children(parent).enumerate() {
            let Some(raw) = store.node(child) else {
                continue;
            };
            if !is_visible(store, raw, opts) {
                continue;
            }
            let Some(node) = self.node(child) else {
                continue;
            };
            scratch.push(Keyed {
                folder_rank: u8::from(!node.is_folder()),
                kind: node.kind,
                value: value_of(self, &node, sort.column, sort.side),
                position,
                id: child,
            });
        }
        let name = sort.column == SortColumn::Name;
        scratch.sort_by(|a, b| {
            // Name keeps the tree's kind rank (Folder, File, Symlink, Other) so that Name ascending is the tree order;
            // the other columns put folders first and compare the value alone.
            let kind = |k: &Keyed<'_>| if name { k.kind as u8 } else { 0 };
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
        stack.extend(scratch.iter().rev().map(|k| (k.id, depth)));
        scratch.clear();
    }
}

/// Compares two rows by `sort` as [`TreeRead::visible_rows_sorted`] does for siblings, without the tree position
/// (tests and the bench: `Equal` means the tree's order decides).
pub fn compare_rows(read: &TreeRead<'_>, a: NodeId, b: NodeId, sort: &SortKey) -> Ordering {
    let (Some(x), Some(y)) = (read.node(a), read.node(b)) else {
        return Ordering::Equal;
    };
    let rank = |n: &NodeView<'_>| u8::from(!n.is_folder());
    let name = sort.column == SortColumn::Name;
    let kind = |n: &NodeView<'_>| if name { n.kind as u8 } else { 0 };
    let by_value = (kind(&x), value_of(read, &x, sort.column, sort.side))
        .cmp(&(kind(&y), value_of(read, &y, sort.column, sort.side)));
    rank(&x).cmp(&rank(&y)).then(if sort.descending {
        by_value.reverse()
    } else {
        by_value
    })
}
