//! The member tree of one archive (D-139, D-141, D-142): validated, ordered, independent of archive order.
//!
//! Format readers hand their members over as [`RawMember`]s, in archive order. [`build_tree`] applies rules 1–3 and
//! 6 of §1.5.7 (the result does not depend on the order) and synthesizes the folders that only exist through their
//! members. Nodes are sorted by (parent, name), so a binary search finds a path and the children of a folder are a
//! range.
//!
//! Archives are hostile and a path can name thousands of folders, so nothing here costs entries × depth × path bytes
//! (D-139): [`NameBudget`] counts the text of an index while it is read, a synthesized folder counts as an entry and
//! shares the text of a member it is a prefix of, each folder is looked at once, and every loop looks at `cancel`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use hikasa_core::{Ctl, FileTime};

use super::codec::Pass;
use super::name::{Refusal, escape_name};
use super::tar_walk::Damage;
use crate::entry::{Attrs, Entry, EntryKind, FileName};
use crate::error::VfsError;

/// The most `<refused-N>` rows of a listing; the warning has the total.
pub(crate) const REFUSED_ROWS_MAX: usize = 1000;
/// Steps of a loop between two looks at `cancel`.
const CANCEL_STEP: usize = 1 << 10;

/// What the text of one index may cost (rule 13): every path, refused name and link target counts, and so does the
/// path of a synthesized folder, which is compared and hashed although it is not stored.
#[derive(Debug)]
pub(crate) struct NameBudget<'a> {
    spec: &'a str,
    paths_max: u64,
    depth_max: u32,
    entries_max: u64,
    used: u64,
}

impl<'a> NameBudget<'a> {
    /// The budget of the pass `pass`.
    pub(crate) fn new(pass: &Pass<'a>) -> NameBudget<'a> {
        NameBudget {
            spec: pass.spec,
            paths_max: pass.limits.paths_max,
            depth_max: pass.limits.depth_max,
            entries_max: pass.limits.entries_max,
            used: 0,
        }
    }

    fn limit(&self, limit: &'static str, value: u64) -> VfsError {
        VfsError::ArchiveLimit {
            spec: self.spec.to_owned(),
            limit,
            value,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), VfsError> {
        self.used = self.used.saturating_add(bytes as u64);
        if self.used > self.paths_max {
            return Err(self.limit("paths", self.paths_max));
        }
        Ok(())
    }

    /// Counts a member before it is kept.
    pub(crate) fn member(&mut self, member: &RawMember) -> Result<(), VfsError> {
        let path = match &member.path {
            Ok(path) => {
                if path.bytes().filter(|&b| b == b'/').count() >= self.depth_max as usize {
                    return Err(self.limit("depth", u64::from(self.depth_max)));
                }
                path
            }
            Err((name, _)) => name,
        };
        self.charge(path.len())?;
        self.charge(member.link_target.as_ref().map_or(0, |target| target.len()))?;
        self.charge(member.hard_target.as_ref().map_or(0, String::len))
    }

    /// Counts a link target that came after [`NameBudget::member`].
    pub(crate) fn member_link(&mut self, member: &RawMember) -> Result<(), VfsError> {
        self.charge(member.link_target.as_ref().map_or(0, |target| target.len()))
    }

    /// Counts a folder that only exists through its members.
    fn folder(&mut self, path: &str, nodes: u64) -> Result<(), VfsError> {
        if nodes > self.entries_max {
            return Err(self.limit("entries", self.entries_max));
        }
        self.charge(path.len())
    }
}

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
    /// The path's text replaced bytes that do not decode.
    pub(crate) lossy: bool,
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
    /// Holds the path as its first `len` bytes: a member's own text, which a synthesized folder shares.
    text: Arc<str>,
    len: u32,
    /// Where the last component starts.
    name_at: u32,
    pub(crate) kind: MemberKind,
    lossy: bool,
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
    fn new(raw: RawMember, path: Arc<str>) -> Node {
        Node {
            name_at: name_start(&path),
            len: u32::try_from(path.len()).unwrap_or(0),
            text: path,
            kind: raw.kind,
            lossy: raw.lossy,
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

    /// A folder that only exists through its members: `path` is a prefix of `text`.
    fn synthesized(text: &Arc<str>, path: &str, lossy: bool) -> Node {
        Node {
            name_at: name_start(path),
            len: u32::try_from(path.len()).unwrap_or(0),
            text: Arc::clone(text),
            kind: MemberKind::Dir,
            lossy,
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

    /// The path.
    pub(crate) fn path(&self) -> &str {
        self.text.get(..self.len as usize).unwrap_or_default()
    }

    /// (parent path, name).
    fn key(&self) -> (&str, &str) {
        split_path(self.path(), self.name_at as usize)
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
        let name = match FileName::new(name) {
            Ok(_) if self.lossy && name.contains(char::REPLACEMENT_CHARACTER) => {
                FileName::from_lossy_text(name)
            }
            Ok(name) => name,
            Err(_) => FileName::from_os(std::ffi::OsStr::new(name)),
        };
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
    /// NUL bytes followed the compressed stream.
    pub(crate) padded: bool,
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

    /// Bytes of the tree, for the budget of §1.5.5. Text shared by a synthesized folder counts once.
    pub(crate) fn memory(&self) -> usize {
        let nodes: usize = self
            .nodes
            .iter()
            .map(|node| {
                let own = if node.len as usize == node.text.len() {
                    node.text.len() + 2 * size_of::<usize>()
                } else {
                    0
                };
                own + node.link_target.as_ref().map_or(0, |target| target.len())
            })
            .sum();
        nodes
            + self.nodes.capacity() * size_of::<Node>()
            + self.records.capacity() * size_of::<Record>()
    }
}

/// Sorts `items` as `sort_by` does, looking at `ctl` between steps: 1,000,000 hostile paths sort for seconds, and G4
/// wants a stop within 100 ms. Runs are sorted, then merged as indexes, so the items do not move twice.
fn sort_checked<T>(
    items: &mut [T],
    compare: impl Fn(&T, &T) -> Ordering,
    ctl: Ctl<'_>,
) -> Result<(), VfsError> {
    const RUN: usize = 1 << 12;
    if items.len() <= RUN {
        items.sort_by(compare);
        return Ok(());
    }
    let by_index = |a: &u32, b: &u32| match (items.get(*a as usize), items.get(*b as usize)) {
        (Some(a), Some(b)) => compare(a, b),
        _ => Ordering::Equal,
    };
    let mut order: Vec<u32> = (0..u32::try_from(items.len()).unwrap_or(u32::MAX)).collect();
    for run in order.chunks_mut(RUN) {
        ctl.check()?;
        run.sort_by(by_index);
    }
    let mut merged = Vec::with_capacity(order.len());
    let mut size = RUN;
    while size < order.len() {
        merged.clear();
        for pair in order.chunks(2 * size) {
            let (left, right) = pair.split_at(size.min(pair.len()));
            let (mut left, mut right) = (left.iter().peekable(), right.iter().peekable());
            loop {
                let next = match (left.peek(), right.peek()) {
                    (Some(a), Some(b)) if by_index(b, a) == Ordering::Less => right.next(),
                    (Some(_), _) => left.next(),
                    (None, _) => right.next(),
                };
                let Some(&next) = next else { break };
                merged.push(next);
                if merged.len().is_multiple_of(RUN) {
                    ctl.check()?;
                }
            }
        }
        std::mem::swap(&mut order, &mut merged);
        size *= 2;
    }
    // `order[i]` names the item that belongs at `i`: each cycle is walked once.
    for start in 0..order.len() {
        let mut at = start;
        while let Some(slot) = order.get_mut(at) {
            let from = *slot as usize;
            *slot = u32::try_from(at).unwrap_or(u32::MAX);
            if from == start {
                break;
            }
            items.swap(at, from);
            at = from;
        }
    }
    Ok(())
}

/// Applies rules 1–3 and 6 to `raw`, which is in archive order and passed `budget`. Nothing here costs more than the
/// budget allows: each distinct folder is looked at once, and a folder that no member names counts in `budget`.
pub(crate) fn build_tree(
    raw: Vec<RawMember>,
    mut budget: NameBudget<'_>,
    pass: &Pass<'_>,
) -> Result<Tree, VfsError> {
    let ctl = pass.ctl;
    let tick = |step: usize| {
        if step.is_multiple_of(CANCEL_STEP) {
            ctl.check()
        } else {
            Ok(())
        }
    };
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

    // Rule 1, and hard links in stream order (rule 6). The text moves out of the members, so it exists once.
    let has_hard_links = raw
        .iter()
        .flatten()
        .any(|member| member.kind == MemberKind::HardLink);
    let mut regular: HashMap<Arc<str>, (u64, Option<u32>, DataRef, usize)> = HashMap::new();
    let mut hard_links: Vec<(usize, usize)> = Vec::new();
    let mut live: Vec<(usize, Arc<str>)> = Vec::with_capacity(raw.len());
    for (at, slot) in raw.iter_mut().enumerate() {
        tick(at)?;
        let Some(member) = slot else { continue };
        let path = match std::mem::replace(&mut member.path, Ok(String::new())) {
            Ok(path) => path,
            Err((name, reason)) => {
                refuse(at, name, reason, &mut states);
                *slot = None;
                continue;
            }
        };
        if member.kind == MemberKind::HardLink {
            let target = member
                .hard_target
                .as_deref()
                .and_then(|target| regular.get(target));
            let Some(&(size, crc32, data, target)) = target else {
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
            hard_links.push((at, target));
        }
        if path.is_empty() {
            // `./` gives the root its metadata; a file of that name has nowhere to go.
            if member.kind == MemberKind::Dir || member.size == 0 {
                tree.root_attrs = Some(member.attrs);
            } else {
                refuse(at, ".".to_owned(), Refusal::Empty, &mut states);
            }
            *slot = None;
            continue;
        }
        let path = Arc::<str>::from(path);
        if has_hard_links && member.kind == MemberKind::File {
            let fact = (member.size, member.crc32, member.data, at);
            regular.insert(Arc::clone(&path), fact);
        }
        live.push((at, path));
    }
    drop(regular);

    // Rules 2 (same path) and 3: for each path, the last folder, else the last member. Names that only look equal
    // because bytes were replaced are two members: the shadowed one is refused, so a rebuild keeps it.
    sort_checked(&mut live, |a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)), ctl)?;
    let mut kept: Vec<(usize, Arc<str>)> = Vec::with_capacity(live.len());
    let kind_of = |raw: &[Option<RawMember>], at: usize| {
        raw.get(at)
            .and_then(Option::as_ref)
            .map_or(MemberKind::Other, |member| member.kind)
    };
    let lossy_at = |raw: &[Option<RawMember>], at: usize| {
        raw.get(at)
            .and_then(Option::as_ref)
            .is_some_and(|member| member.lossy)
    };
    let mut group = Vec::new();
    let mut live = live.into_iter().peekable();
    let mut step = 0;
    while let Some((at, path)) = live.next() {
        step += 1;
        tick(step)?;
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
        let twins = group.iter().any(|&at| lossy_at(&raw, at));
        for &at in &group {
            if at == winner {
                continue;
            }
            if twins || (folder.is_some() && kind_of(&raw, at) != MemberKind::Dir) {
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
    // refused; a file that is a parent is refused and the folder stays. `below_link` knows each folder once.
    let lookup = |path: &str| {
        kept.binary_search_by(|(_, other)| (**other).cmp(path))
            .ok()
            .and_then(|at| kept.get(at))
            .map(|(at, _)| *at)
    };
    let mut below_link: HashMap<&str, bool> = HashMap::new();
    let mut gone: HashSet<usize> = HashSet::new();
    let mut folders_new = Vec::new();
    let mut nodes_total = kept.len() as u64;
    for (step, (at, path)) in kept.iter().enumerate() {
        tick(step)?;
        let (mut parent, _) = path_key(path);
        let mut below = false;
        folders_new.clear();
        while !parent.is_empty() {
            if let Some(&known) = below_link.get(parent) {
                below = known;
                break;
            }
            folders_new.push(parent);
            parent = path_key(parent).0;
        }
        for &folder in folders_new.iter().rev() {
            match lookup(folder) {
                None => {
                    nodes_total += 1;
                    budget.folder(folder, nodes_total)?;
                }
                Some(_) if below => {}
                Some(ancestor) => match kind_of(&raw, ancestor) {
                    MemberKind::Dir => {}
                    MemberKind::Symlink => below = true,
                    _ => {
                        gone.insert(ancestor);
                    }
                },
            }
            below_link.insert(folder, below);
        }
        if below {
            gone.insert(*at);
        }
    }
    drop(below_link);

    // Rule 6 again: a hard link needs a member that stayed.
    let mut broken: HashSet<usize> = HashSet::new();
    for &(at, target) in &hard_links {
        if gone.contains(&target) || broken.contains(&target) {
            broken.insert(at);
        }
    }
    gone.extend(&broken);

    // The tree.
    let mut folders: HashSet<&str> = HashSet::new();
    let mut nodes: Vec<Node> = Vec::with_capacity(kept.len());
    for (step, (at, path)) in kept.iter().enumerate() {
        tick(step)?;
        if gone.contains(at) {
            continue;
        }
        let lossy = lossy_at(&raw, *at);
        let (mut parent, _) = path_key(path);
        while !parent.is_empty() && folders.insert(parent) {
            let survives = lookup(parent).is_some_and(|at| !gone.contains(&at));
            if !survives {
                nodes.push(Node::synthesized(path, parent, lossy));
            }
            parent = path_key(parent).0;
        }
    }
    drop(folders);
    for (step, (at, path)) in kept.iter().enumerate() {
        tick(step)?;
        let Some(member) = raw.get_mut(*at).and_then(Option::take) else {
            continue;
        };
        if gone.contains(at) {
            let reason = if broken.contains(at) {
                Refusal::HardLink
            } else {
                Refusal::Conflict
            };
            refuse(*at, escape_name(path.as_bytes()), reason, &mut states);
            continue;
        }
        tree.encrypted += u64::from(member.encrypted);
        nodes.push(Node::new(member, Arc::clone(path)));
    }
    sort_checked(&mut nodes, |a, b| a.key().cmp(&b.key()), ctl)?;
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
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ArchiveLimits;

    fn raw(path: &str, kind: MemberKind, id: u64) -> RawMember {
        RawMember {
            path: Ok(path.to_owned()),
            kind,
            lossy: false,
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

    fn built(members: Vec<RawMember>, limits: ArchiveLimits) -> Result<Tree, VfsError> {
        let pass = Pass {
            spec: "t",
            limits,
            ctl: Ctl::detached(),
            zone: jiff::tz::TimeZone::UTC,
        };
        let mut budget = NameBudget::new(&pass);
        for member in &members {
            budget.member(member)?;
        }
        build_tree(members, budget, &pass)
    }

    fn tree_of(members: Vec<RawMember>) -> Tree {
        built(members, ArchiveLimits::default()).unwrap()
    }

    fn shape(tree: &Tree) -> Vec<(String, MemberKind, u64)> {
        tree.nodes
            .iter()
            .map(|n| (n.path().to_owned(), n.kind, n.size))
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
        let want = shape(&tree_of(make()));
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
            let tree = tree_of(order);
            assert_eq!(shape(&tree), want, "rotation {rotation}");
            assert_eq!(tree.refused_total, 2);
        }
    }

    #[test]
    fn duplicates_last_wins_and_records_are_kept() {
        let tree = tree_of(vec![file("a", 1), file("b", 2), file("a", 3)]);
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
        let tree = tree_of(vec![file("x/y", 1), file("x-y", 2), file("x/z", 3)]);
        assert!(tree.find("x/y").is_some());
        assert_eq!(tree.find("x").unwrap().kind, MemberKind::Dir);
        assert!(tree.find("nope").is_none());
        let names: Vec<_> = tree.children("x").iter().map(Node::path).collect();
        assert_eq!(names, ["x/y", "x/z"]);
        assert_eq!(tree.children("").len(), 2);
        assert!(tree.blocked("x-y/z"));
        assert!(!tree.blocked("x/q"));
    }

    /// The figure of §1.5.5.
    #[test]
    fn a_node_and_a_record_stay_small() {
        assert!(size_of::<Node>() + size_of::<Record>() <= 200);
    }

    /// Every size of input gives what `sort_by` gives, stable, also past one run.
    #[test]
    fn sort_checked_is_sort_by() {
        for len in [0usize, 1, 2, 4096, 4097, 9000, 20_011] {
            let mut items: Vec<(u32, usize)> = (0..len)
                .map(|at| {
                    let spread = at.wrapping_mul(2_654_435_761) >> 7;
                    (u32::try_from(spread % 512).unwrap(), at)
                })
                .collect();
            let mut want = items.clone();
            want.sort_by_key(|item| item.0);
            sort_checked(&mut items, |a, b| a.0.cmp(&b.0), Ctl::detached()).unwrap();
            assert_eq!(items, want, "{len}");
        }
    }

    /// A folder has its text once, however many folders share a member's path.
    #[test]
    fn a_synthesized_folder_shares_a_members_text() {
        let path = vec!["segment"; 100].join("/");
        let tree = tree_of(vec![file(&path, 1)]);
        assert_eq!(tree.nodes.len(), 100);
        let text: usize = tree.memory() - tree.nodes.capacity() * size_of::<Node>();
        assert!(text < 2 * path.len(), "{text}");
        let deep = tree.find(&path).unwrap();
        assert_eq!(deep.path(), path);
        assert_eq!(tree.find("segment/segment").unwrap().kind, MemberKind::Dir);
    }

    #[test]
    fn synthesized_folders_count_as_entries() {
        let limits = ArchiveLimits {
            entries_max: 4,
            ..ArchiveLimits::default()
        };
        let fits = vec![file("a/b/c/d", 1)];
        assert_eq!(built(fits, limits).unwrap().nodes.len(), 4);
        let over = built(vec![file("a/b/c/d/e", 1)], limits);
        assert!(
            matches!(
                over,
                Err(VfsError::ArchiveLimit {
                    limit: "entries",
                    value: 4,
                    ..
                })
            ),
            "{over:?}"
        );
    }

    #[test]
    fn folder_paths_count_in_the_budget() {
        let limits = ArchiveLimits {
            paths_max: 16,
            ..ArchiveLimits::default()
        };
        let fits = vec![file("aaa/bbbbb", 1)];
        assert!(built(fits, limits).is_ok(), "9 bytes and a folder of 3");
        // 13 bytes of path and three folders of 3, 7 and 11: 34.
        let over = built(vec![file("aaa/bbb/ccc/d", 1)], limits);
        assert!(
            matches!(
                over,
                Err(VfsError::ArchiveLimit {
                    limit: "paths",
                    value: 16,
                    ..
                })
            ),
            "{over:?}"
        );
    }

    #[test]
    fn the_depth_is_capped() {
        let limits = ArchiveLimits {
            depth_max: 3,
            ..ArchiveLimits::default()
        };
        assert!(built(vec![file("a/b/c", 1)], limits).is_ok());
        let over = built(vec![file("a/b/c/d", 1)], limits);
        assert!(
            matches!(
                over,
                Err(VfsError::ArchiveLimit {
                    limit: "depth",
                    value: 3,
                    ..
                })
            ),
            "{over:?}"
        );
    }

    #[test]
    fn a_cancelled_tree_stops() {
        let cancel = hikasa_core::CancelToken::new();
        cancel.cancel();
        let pass = Pass {
            spec: "t",
            limits: ArchiveLimits::default(),
            ctl: Ctl {
                cancel: &cancel,
                progress: &hikasa_core::NullProgress,
            },
            zone: jiff::tz::TimeZone::UTC,
        };
        let tree = build_tree(vec![file("a", 1)], NameBudget::new(&pass), &pass);
        assert!(matches!(tree, Err(VfsError::Cancelled)), "{tree:?}");
    }

    /// Rule 6: a link to a member that rule 2 refused is refused, also through another link.
    #[test]
    fn a_hard_link_needs_a_member_that_stayed() {
        let hard = |path: &str, target: &str, id| RawMember {
            hard_target: Some(target.to_owned()),
            ..raw(path, MemberKind::HardLink, id)
        };
        let tree = tree_of(vec![
            raw("lnk", MemberKind::Symlink, 1),
            file("lnk/x", 2),
            hard("h1", "lnk/x", 3),
            hard("h2", "h1", 4),
            file("ok", 5),
            hard("h3", "ok", 6),
        ]);
        let names: Vec<_> = tree.nodes.iter().map(Node::path).collect();
        assert_eq!(names, ["h3", "lnk", "ok"]);
        let reasons: Vec<_> = tree
            .refused
            .iter()
            .map(|r| (r.name.as_str(), r.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("lnk/x", Refusal::Conflict),
                ("h1", Refusal::HardLink),
                ("h2", Refusal::HardLink)
            ]
        );
    }

    /// A file named `.` has data and nowhere to go; a folder of that name only gives the root its metadata.
    #[test]
    fn a_member_that_reduces_to_nothing() {
        let tree = tree_of(vec![raw("", MemberKind::Dir, 0), file("", 3)]);
        assert!(tree.nodes.is_empty());
        assert_eq!(tree.refused_total, 1);
        assert_eq!(tree.refused[0].reason, Refusal::Empty);
        assert!(tree.root_attrs.is_some());
    }

    /// Two names that only look equal after a lossy decode are two members: neither is a duplicate to drop.
    #[test]
    fn lossy_twins_are_refused_not_shadowed() {
        let twin = |id| RawMember {
            lossy: true,
            ..file("a\u{FFFD}", id)
        };
        let tree = tree_of(vec![twin(1), twin(2), file("b", 3)]);
        assert_eq!(tree.duplicates, 0);
        assert_eq!(tree.refused_total, 1);
        assert_eq!(tree.find("a\u{FFFD}").unwrap().size, 2);
        let states: Vec<_> = tree.records.iter().map(|r| r.state).collect();
        assert_eq!(
            states,
            [RecordState::Refused, RecordState::Live, RecordState::Live]
        );
        let name = tree.find("a\u{FFFD}").unwrap().entry().name;
        assert!(name.is_lossy());
        assert_eq!(name.as_str(), "a\u{FFFD}");
    }
}
