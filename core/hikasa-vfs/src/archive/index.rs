//! The member tree of one archive (D-139, D-141, D-142): validated, ordered, independent of archive order.
//!
//! Format readers hand their members over as [`RawMember`]s, in archive order. [`build_tree`] applies rules 1–3 and
//! 6 of §1.5.7 (the result does not depend on the order) and synthesizes the folders that only exist through their
//! members. Nodes are sorted by (parent, name), so a binary search finds a path and the children of a folder are a
//! range.

use std::collections::{HashMap, HashSet};

use hikasa_core::FileTime;

use super::name::{Refusal, escape_name};
use super::tar_walk::Damage;
use crate::entry::{Attrs, Entry, EntryKind, FileName};

/// The most `<refused-N>` rows of a listing; the warning has the total.
pub(crate) const REFUSED_ROWS_MAX: usize = 1000;

/// What a member is, before and after hard links resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Dir,
    File,
    Symlink,
    /// Only in a [`RawMember`]: resolved or refused by [`build_tree`].
    HardLink,
    Other,
    /// Listed, never read.
    Sparse,
}

/// Where a member's data lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataRef {
    /// Nothing (a synthesized folder).
    None,
    /// An entry of the zip's central directory.
    Zip { index: usize },
    /// Blocks of a tar stream.
    Tar {
        first_block: u64,
        data: u64,
        end: u64,
    },
    /// The whole decoded stream.
    Stream,
}

/// One member as a format reader saw it.
#[derive(Debug)]
pub(crate) struct RawMember {
    /// The clean path, or the escaped raw name and why it is refused.
    pub(crate) path: Result<String, (String, Refusal)>,
    pub(crate) kind: MemberKind,
    pub(crate) encrypted: bool,
    pub(crate) size: u64,
    pub(crate) mtime: Option<FileTime>,
    pub(crate) ctime: Option<FileTime>,
    pub(crate) attrs: Attrs,
    pub(crate) crc32: Option<u32>,
    /// The target text of a link.
    pub(crate) link_target: Option<Box<str>>,
    /// The clean path a hard link names (`None` when that name is refused).
    pub(crate) hard_target: Option<String>,
    pub(crate) data: DataRef,
}

/// One node of the tree.
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(crate) path: Box<str>,
    /// Where the last component starts.
    name_at: u32,
    pub(crate) kind: MemberKind,
    pub(crate) encrypted: bool,
    pub(crate) size: u64,
    pub(crate) mtime: Option<FileTime>,
    pub(crate) ctime: Option<FileTime>,
    pub(crate) attrs: Attrs,
    pub(crate) crc32: Option<u32>,
    pub(crate) link_target: Option<Box<str>>,
    pub(crate) data: DataRef,
}

impl Node {
    fn new(raw: RawMember, path: String) -> Node {
        Node {
            name_at: name_start(&path),
            path: path.into_boxed_str(),
            kind: raw.kind,
            encrypted: raw.encrypted,
            size: raw.size,
            mtime: raw.mtime,
            ctime: raw.ctime,
            attrs: raw.attrs,
            crc32: raw.crc32,
            link_target: raw.link_target,
            data: raw.data,
        }
    }

    /// A folder that only exists through its members.
    fn synthesized(path: &str) -> Node {
        Node {
            name_at: name_start(path),
            path: path.into(),
            kind: MemberKind::Dir,
            encrypted: false,
            size: 0,
            mtime: None,
            ctime: None,
            attrs: Attrs::None,
            crc32: None,
            link_target: None,
            data: DataRef::None,
        }
    }

    /// (parent path, name).
    fn key(&self) -> (&str, &str) {
        split_path(&self.path, self.name_at as usize)
    }

    /// The entry of this node.
    pub(crate) fn entry(&self) -> Entry {
        let (_, name) = self.key();
        let kind = match self.kind {
            MemberKind::Dir => EntryKind::Dir,
            MemberKind::File | MemberKind::HardLink | MemberKind::Sparse => EntryKind::File,
            MemberKind::Symlink => EntryKind::Symlink,
            MemberKind::Other => EntryKind::Other,
        };
        let name =
            FileName::new(name).unwrap_or_else(|_| FileName::from_os(std::ffi::OsStr::new(name)));
        let mut entry = Entry::new(name, kind);
        entry.size = self.size;
        entry.mtime = self.mtime;
        entry.ctime = self.ctime;
        entry.attrs = self.attrs;
        entry.link_target = self.link_target.as_deref().map(str::to_owned);
        entry.crc32 = self.crc32;
        entry
    }
}

fn name_start(path: &str) -> u32 {
    path.rfind('/')
        .map_or(0, |at| u32::try_from(at + 1).unwrap_or(0))
}

fn split_path(path: &str, name_at: usize) -> (&str, &str) {
    if name_at == 0 {
        return ("", path);
    }
    let parent = path.get(..name_at - 1).unwrap_or("");
    let name = path.get(name_at..).unwrap_or("");
    (parent, name)
}

/// The (parent, name) key of a path.
fn path_key(path: &str) -> (&str, &str) {
    split_path(path, name_start(path) as usize)
}

/// What became of a record of the archive; a rebuild (W5b) keeps `Refused` and drops `Shadowed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum RecordState {
    Live,
    Refused,
    Shadowed,
}

/// One record of the archive, in archive order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Record {
    pub(crate) data: DataRef,
    pub(crate) state: RecordState,
}

/// One `<refused-N>` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefusedRow {
    pub(crate) name: String,
    pub(crate) reason: Refusal,
}

/// The result of [`build_tree`].
#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub(crate) nodes: Vec<Node>,
    /// Rows in archive order, at most [`REFUSED_ROWS_MAX`].
    pub(crate) refused: Vec<RefusedRow>,
    pub(crate) refused_total: u64,
    pub(crate) duplicates: u64,
    pub(crate) encrypted: u64,
    /// The attributes a `./` entry gives the root.
    pub(crate) root_attrs: Option<Attrs>,
    /// Every record in archive order (the seam of W5b).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) records: Vec<Record>,
    /// Set when the stream was damaged.
    pub(crate) damage: Option<Damage>,
}

impl Tree {
    /// The node of `path`.
    pub(crate) fn find(&self, path: &str) -> Option<&Node> {
        let key = path_key(path);
        self.nodes
            .binary_search_by(|node| node.key().cmp(&key))
            .ok()
            .and_then(|at| self.nodes.get(at))
    }

    /// The children of the folder `path` (`""` is the root).
    pub(crate) fn children(&self, path: &str) -> &[Node] {
        let start = self.nodes.partition_point(|node| node.key().0 < path);
        let end = self.nodes.partition_point(|node| node.key().0 <= path);
        self.nodes.get(start..end).unwrap_or(&[])
    }

    /// Why `path` cannot be looked up: `true` when a component on the way is not a folder.
    pub(crate) fn blocked(&self, path: &str) -> bool {
        path.match_indices('/').any(|(at, _)| {
            path.get(..at)
                .and_then(|prefix| self.find(prefix))
                .is_some_and(|node| node.kind != MemberKind::Dir)
        })
    }

    /// Bytes of the tree, for the budget of §1.5.5.
    pub(crate) fn memory(&self) -> usize {
        let nodes: usize = self
            .nodes
            .iter()
            .map(|node| {
                node.path.len() + node.link_target.as_ref().map_or(0, |target| target.len())
            })
            .sum();
        nodes
            + self.nodes.capacity() * size_of::<Node>()
            + self.records.capacity() * size_of::<Record>()
    }
}

/// Applies rules 1–3 and 6 to `raw`, which is in archive order.
pub(crate) fn build_tree(raw: Vec<RawMember>) -> Tree {
    let mut tree = Tree::default();
    let mut refused: Vec<(usize, RefusedRow)> = Vec::new();
    let mut states = vec![RecordState::Live; raw.len()];
    let mut raw: Vec<Option<RawMember>> = raw.into_iter().map(Some).collect();
    tree.records.reserve_exact(raw.len());
    tree.records
        .extend(raw.iter().flatten().map(|member| Record {
            data: member.data,
            state: RecordState::Live,
        }));
    let mut refuse = |at: usize, name: String, reason: Refusal, states: &mut [RecordState]| {
        if let Some(state) = states.get_mut(at) {
            *state = RecordState::Refused;
        }
        tree.refused_total += 1;
        refused.push((at, RefusedRow { name, reason }));
        // Hostile names can be long and many: only the first rows in archive order are ever shown.
        if refused.len() >= 4 * REFUSED_ROWS_MAX {
            refused.sort_by_key(|(at, _)| *at);
            refused.truncate(REFUSED_ROWS_MAX);
        }
    };

    // Rule 1, and hard links in stream order (rule 6).
    let has_hard_links = raw
        .iter()
        .flatten()
        .any(|member| member.kind == MemberKind::HardLink);
    let mut regular: HashMap<String, (u64, Option<u32>, DataRef)> = HashMap::new();
    let mut live: Vec<(usize, String)> = Vec::with_capacity(raw.len());
    for (at, slot) in raw.iter_mut().enumerate() {
        let Some(member) = slot else { continue };
        let path = match &member.path {
            Ok(path) => path.clone(),
            Err((name, reason)) => {
                refuse(at, name.clone(), *reason, &mut states);
                *slot = None;
                continue;
            }
        };
        if member.kind == MemberKind::HardLink {
            let target = member
                .hard_target
                .as_ref()
                .and_then(|target| regular.get(target));
            let Some(&(size, crc32, data)) = target else {
                refuse(
                    at,
                    escape_name(path.as_bytes()),
                    Refusal::HardLink,
                    &mut states,
                );
                *slot = None;
                continue;
            };
            member.kind = MemberKind::File;
            member.size = size;
            member.crc32 = crc32;
            member.data = data;
        }
        if path.is_empty() {
            tree.root_attrs = Some(member.attrs);
            *slot = None;
            continue;
        }
        if has_hard_links && member.kind == MemberKind::File {
            regular.insert(path.clone(), (member.size, member.crc32, member.data));
        }
        live.push((at, path));
    }

    // Rules 2 (same path) and 3: for each path, the last folder, else the last member.
    live.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut kept: Vec<(usize, String)> = Vec::with_capacity(live.len());
    let kind_of = |raw: &[Option<RawMember>], at: usize| {
        raw.get(at)
            .and_then(Option::as_ref)
            .map_or(MemberKind::Other, |member| member.kind)
    };
    let mut group = Vec::new();
    let mut live = live.into_iter().peekable();
    while let Some((at, path)) = live.next() {
        group.clear();
        group.push(at);
        while let Some((next, _)) = live.next_if(|(_, other)| *other == path) {
            group.push(next);
        }
        let folder = group
            .iter()
            .copied()
            .rfind(|&at| kind_of(&raw, at) == MemberKind::Dir);
        let last = group.last().copied().unwrap_or(at);
        let winner = folder.unwrap_or(last);
        for &at in &group {
            if at == winner {
                continue;
            }
            if folder.is_some() && kind_of(&raw, at) != MemberKind::Dir {
                refuse(
                    at,
                    escape_name(path.as_bytes()),
                    Refusal::Conflict,
                    &mut states,
                );
            } else {
                if let Some(state) = states.get_mut(at) {
                    *state = RecordState::Shadowed;
                }
                tree.duplicates += 1;
            }
            if let Some(slot) = raw.get_mut(at) {
                *slot = None;
            }
        }
        kept.push((winner, path));
    }

    // Rule 2, across paths: `kept` is sorted by path, so a binary search finds an ancestor. A member below a link is
    // refused; a file that is a parent is refused and the folder stays.
    let lookup = |path: &str| {
        kept.binary_search_by(|(_, other)| other.as_str().cmp(path))
            .ok()
            .and_then(|at| kept.get(at))
            .map(|(at, _)| *at)
    };
    let mut parents: HashMap<&str, bool> = HashMap::new();
    let mut gone: HashSet<usize> = HashSet::new();
    for (at, path) in &kept {
        let (parent, _) = path_key(path);
        if parent.is_empty() {
            continue;
        }
        let below_link = *parents.entry(parent).or_insert_with(|| {
            for prefix in parent
                .match_indices('/')
                .map(|(end, _)| parent.get(..end).unwrap_or(parent))
                .chain([parent])
            {
                let Some(ancestor) = lookup(prefix) else {
                    continue;
                };
                match kind_of(&raw, ancestor) {
                    MemberKind::Dir => {}
                    MemberKind::Symlink => return true,
                    _ => {
                        gone.insert(ancestor);
                    }
                }
            }
            false
        });
        if below_link {
            gone.insert(*at);
        }
    }

    // The tree.
    let mut folders: HashSet<&str> = HashSet::new();
    for (at, path) in &kept {
        if gone.contains(at) {
            continue;
        }
        let (mut parent, _) = path_key(path);
        while !parent.is_empty() && folders.insert(parent) {
            parent = path_key(parent).0;
        }
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(kept.len() + folders.len());
    for folder in &folders {
        let survives = lookup(folder).is_some_and(|at| !gone.contains(&at));
        if !survives {
            nodes.push(Node::synthesized(folder));
        }
    }
    drop(folders);
    for (at, path) in &kept {
        let Some(member) = raw.get_mut(*at).and_then(Option::take) else {
            continue;
        };
        if gone.contains(at) {
            refuse(
                *at,
                escape_name(path.as_bytes()),
                Refusal::Conflict,
                &mut states,
            );
            continue;
        }
        tree.encrypted += u64::from(member.encrypted);
        nodes.push(Node::new(member, path.clone()));
    }
    nodes.sort_by(|a, b| a.key().cmp(&b.key()));
    nodes.shrink_to_fit();
    tree.nodes = nodes;
    for (record, state) in tree.records.iter_mut().zip(&states) {
        record.state = *state;
    }
    refused.sort_by_key(|(at, _)| *at);
    tree.refused = refused
        .into_iter()
        .take(REFUSED_ROWS_MAX)
        .map(|(_, row)| row)
        .collect();
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(path: &str, kind: MemberKind, id: u64) -> RawMember {
        RawMember {
            path: Ok(path.to_owned()),
            kind,
            encrypted: false,
            size: id,
            mtime: None,
            ctime: None,
            attrs: Attrs::None,
            crc32: None,
            link_target: None,
            hard_target: None,
            data: DataRef::Zip {
                index: usize::try_from(id).unwrap(),
            },
        }
    }

    fn file(path: &str, id: u64) -> RawMember {
        raw(path, MemberKind::File, id)
    }

    fn shape(tree: &Tree) -> Vec<(String, MemberKind, u64)> {
        tree.nodes
            .iter()
            .map(|n| (n.path.to_string(), n.kind, n.size))
            .collect()
    }

    /// Every order of the same members gives the same tree (rules 2 and 6's independence).
    #[test]
    fn order_does_not_matter() {
        let make = || {
            vec![
                file("a", 1),
                file("a/b", 2),
                raw("lnk", MemberKind::Symlink, 3),
                file("lnk/x", 4),
                file("d/e/f", 5),
                raw("d/", MemberKind::Dir, 0),
            ]
            .into_iter()
            .map(|mut m| {
                m.path = m.path.map(|p| p.trim_end_matches('/').to_owned());
                m
            })
            .collect::<Vec<_>>()
        };
        let want = shape(&build_tree(make()));
        assert_eq!(
            want,
            [
                ("a".to_owned(), MemberKind::Dir, 0),
                ("d".to_owned(), MemberKind::Dir, 0),
                ("lnk".to_owned(), MemberKind::Symlink, 3),
                ("a/b".to_owned(), MemberKind::File, 2),
                ("d/e".to_owned(), MemberKind::Dir, 0),
                ("d/e/f".to_owned(), MemberKind::File, 5),
            ]
        );
        for rotation in 0..make().len() {
            let mut order = make();
            order.rotate_left(rotation);
            let tree = build_tree(order);
            assert_eq!(shape(&tree), want, "rotation {rotation}");
            assert_eq!(tree.refused_total, 2);
        }
    }

    #[test]
    fn duplicates_last_wins_and_records_are_kept() {
        let tree = build_tree(vec![file("a", 1), file("b", 2), file("a", 3)]);
        assert_eq!(tree.duplicates, 1);
        assert_eq!(tree.find("a").unwrap().size, 3);
        let states: Vec<_> = tree.records.iter().map(|r| r.state).collect();
        assert_eq!(
            states,
            [RecordState::Shadowed, RecordState::Live, RecordState::Live]
        );
    }

    #[test]
    fn lookups() {
        let tree = build_tree(vec![file("x/y", 1), file("x-y", 2), file("x/z", 3)]);
        assert!(tree.find("x/y").is_some());
        assert_eq!(tree.find("x").unwrap().kind, MemberKind::Dir);
        assert!(tree.find("nope").is_none());
        let names: Vec<_> = tree.children("x").iter().map(|n| &*n.path).collect();
        assert_eq!(names, ["x/y", "x/z"]);
        assert_eq!(tree.children("").len(), 2);
        assert!(tree.blocked("x-y/z"));
        assert!(!tree.blocked("x/q"));
    }
}
