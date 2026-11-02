//! The plan of a rename (§1.6.13): each resolved row is one `rename` of the row itself. A rename never replaces; chains are
//! ordered, the free end first, and a cycle is broken by a hidden name.

use std::collections::{BTreeMap, BTreeSet};

use hikasa_core::{ErrorKind, Side, Spec};
use hikasa_vfs::ops::{Action, Expect, PlanItem, SkipReason};
use hikasa_vfs::{Capabilities, EntryKind, FileName, VfsError};

use super::build::{Builder, key_of, name_problem, nothing_on, report, report_with, stamp_of};
use super::conflict::Conflicts;
use super::rename::{RegexRename, dos_rename};
use super::{ItemMeta, OpScope, RenameMask};
use crate::error::FolderError;
use crate::model::{NodeFlags, NodeId};
use crate::select::{Resolved, SideSet};

/// The suffix of a hidden name.
const HIDDEN: &str = ".hikasa-ren";
/// The longest name a file system is sure to hold.
const NAME_MAX: usize = 255;

/// How names are made.
enum Namer<'m> {
    /// One name.
    Name(&'m str),
    /// A DOS mask.
    Dos(&'m str),
    /// A regular expression.
    Regex(Box<RegexRename>),
}

impl Namer<'_> {
    /// The new name of `name`: `Ok(None)` without a match.
    fn apply(&self, name: &str) -> Result<Option<String>, FolderError> {
        match self {
            Namer::Name(new) => Ok(Some((*new).to_owned())),
            Namer::Dos(mask) => Ok(Some(dos_rename(name, mask))),
            Namer::Regex(rename) => rename.apply(name),
        }
    }
}

/// One rename that goes on.
#[derive(Debug, Clone)]
struct Ren {
    id: NodeId,
    source: Spec,
    target: Spec,
    kind: EntryKind,
    /// The name it leaves and the one it takes, as the folder compares them.
    from_key: String,
    to_key: String,
    /// `None` when the row has no stamp (a hidden copy).
    stamped: Option<Expect>,
    size: Option<u64>,
    /// Still planned, in plan order.
    live: bool,
}

/// The rows of one folder on one side.
struct Folder {
    parent: Option<Spec>,
    /// The folder's names, as compared.
    taken: BTreeSet<String>,
    rows: Vec<Ren>,
}

impl Builder<'_, '_> {
    /// `Rename`: per side, per folder.
    pub(super) fn rename(
        &mut self,
        rows: &[Resolved],
        scope: &OpScope,
        sides: SideSet,
        mask: &RenameMask,
    ) -> Result<(), FolderError> {
        let namer = match mask {
            RenameMask::Name(name) => Namer::Name(name),
            RenameMask::Dos(mask) => Namer::Dos(mask),
            RenameMask::Regex { old, new } => Namer::Regex(Box::new(RegexRename::new(old, new)?)),
        };
        let picked: Vec<(Side, NodeId)> = sides
            .sides()
            .flat_map(|side| {
                rows.iter()
                    .filter(move |row| {
                        row.sides.contains(SideSet::of(side))
                            && scope.selection.get(row.node).contains(SideSet::of(side))
                    })
                    .map(move |row| (side, row.node))
            })
            .collect();
        if picked.is_empty() {
            return Err(FolderError::InvalidRules(nothing_on(sides)));
        }
        if matches!(mask, RenameMask::Name(_)) && picked.len() > 1 {
            return Err(FolderError::InvalidRules(
                "a new name needs one row: use a mask for several".to_owned(),
            ));
        }
        for side in sides.sides() {
            let mut folders: BTreeMap<NodeId, Folder> = BTreeMap::new();
            for (_, id) in picked.iter().filter(|(on, _)| *on == side) {
                self.ctl.check()?;
                self.rename_row(side, *id, &namer, &mut folders);
            }
            // What lies deepest goes first: a folder renamed before its rows would take their paths with it.
            let mut ordered: Vec<(usize, NodeId, Folder)> = folders
                .into_iter()
                .map(|(parent, folder)| (self.read.ancestors(parent).len(), parent, folder))
                .collect();
            ordered.sort_by_key(|(depth, parent, _)| (std::cmp::Reverse(*depth), *parent));
            for (_, _, folder) in ordered {
                self.rename_folder(side, folder);
            }
        }
        Ok(())
    }

    /// The names of a folder on `side`, as compared.
    fn names_of(&mut self, parent: NodeId, side: Side, at: &Spec) -> BTreeSet<String> {
        let fold = self.folds(at);
        let read = self.read;
        read.children(parent)
            .filter_map(|child| {
                let view = read.node(child)?;
                let name = view.side(side)?.name;
                Some(key_of(name, fold))
            })
            .collect()
    }

    /// One selected row: its new name, the checks, or the item that says why it stays.
    fn rename_row(
        &mut self,
        side: Side,
        id: NodeId,
        namer: &Namer<'_>,
        folders: &mut BTreeMap<NodeId, Folder>,
    ) {
        let read = self.read;
        let Some(view) = read.node(id) else { return };
        let (Some(entry), Some(spec)) = (view.side(side).copied(), read.spec(id, side)) else {
            return;
        };
        let kind = if entry.followed {
            EntryKind::Symlink
        } else {
            view.kind.entry_kind()
        };
        // An opened archive is renamed as the file it is.
        let opened = view.flags.contains(NodeFlags::archive_root(side));
        let source = match (&spec, opened) {
            (Spec::ArchiveMember { archive, .. }, true) => (**archive).clone(),
            _ => spec,
        };
        let kind = if opened { EntryKind::File } else { kind };
        if let Some(reason) = self.refusal(id, &view, None, &[side]) {
            return self.skip_row(id, kind, None, source, 0, reason, Some(side));
        }
        if self.lacks(&source, Capabilities::RENAME, "rename", Some(side)) {
            let reason = SkipReason::Capability;
            return self.skip_row(id, kind, None, source, 0, reason, Some(side));
        }
        let failed = |builder: &mut Self, error| {
            builder.fail_row(
                id,
                Action::Rename,
                kind,
                Some(source.clone()),
                source.clone(),
                0,
                error,
                Some(side),
            );
        };
        if entry.name_lossy {
            let error = report_with(
                ErrorKind::InvalidInput,
                &source,
                format!("{}: the name is not valid UTF-8", source.canonical()),
            );
            return failed(self, error);
        }
        let name = match namer.apply(entry.name) {
            Ok(Some(name)) => name,
            Ok(None) => {
                let reason = SkipReason::NoMatch;
                return self.skip_row(id, kind, None, source, 0, reason, Some(side));
            }
            Err(e) => {
                let error = report_with(ErrorKind::InvalidInput, &source, e.to_string());
                return failed(self, error);
            }
        };
        if name == entry.name {
            let reason = SkipReason::SameName;
            return self.skip_row(id, kind, None, source, 0, reason, Some(side));
        }
        let Some(parent_spec) = source.parent() else {
            let error = report(&VfsError::InvalidTarget {
                spec: source.canonical(),
                reason: "is a root folder",
            });
            return failed(self, error);
        };
        let target = match name_problem(&name)
            .map_err(|why| VfsError::InvalidTarget {
                spec: name.clone(),
                reason: why,
            })
            .and_then(|()| FileName::new(&name))
            .and_then(|file| self.vfs.child(&parent_spec, &file))
        {
            Ok(target) => target,
            Err(e) => {
                let error = report_with(ErrorKind::InvalidInput, &source, e.to_string());
                return failed(self, error);
            }
        };
        let fold = self.folds(&parent_spec);
        let parent = view.parent.unwrap_or(NodeId::ROOT);
        let taken = match folders.get(&parent) {
            Some(_) => BTreeSet::new(),
            None => self.names_of(parent, side, &parent_spec),
        };
        let folder = folders.entry(parent).or_insert(Folder {
            parent: Some(parent_spec),
            taken,
            rows: Vec::new(),
        });
        // A rename acts on the link of a followed row, and what a name-only row or an archive's file was, no row says.
        let stamp = (!entry.stat_skipped && !opened).then(|| Expect {
            source: Some(stamp_of(&entry, false)),
            target: None,
        });
        folder.rows.push(Ren {
            id,
            source,
            target,
            kind,
            from_key: key_of(entry.name, fold),
            to_key: key_of(&name, fold),
            stamped: stamp,
            size: (kind == EntryKind::File).then_some(entry.size),
            live: true,
        });
    }

    /// The collision table of §1.6.13, for one folder.
    fn rename_folder(&mut self, side: Side, mut folder: Folder) {
        let already = |to: &Spec| {
            report(&VfsError::AlreadyExists {
                spec: to.canonical(),
            })
        };
        // Two items reach one name: the first in plan order goes on.
        let mut seen = BTreeSet::new();
        let mut errors: BTreeMap<usize, hikasa_core::ErrorReport> = BTreeMap::new();
        for (index, ren) in folder.rows.iter_mut().enumerate() {
            if !seen.insert(ren.to_key.clone()) {
                ren.live = false;
                errors.insert(index, already(&ren.target));
            }
        }
        // A name held by what does not move: going on would replace it. One that fails holds its own name in turn.
        loop {
            let moving: BTreeSet<&str> = folder
                .rows
                .iter()
                .filter(|ren| ren.live)
                .map(|ren| ren.from_key.as_str())
                .collect();
            let blocked: Vec<usize> = folder
                .rows
                .iter()
                .enumerate()
                .filter(|(_, ren)| {
                    ren.live
                        && ren.to_key != ren.from_key
                        && folder.taken.contains(&ren.to_key)
                        && !moving.contains(ren.to_key.as_str())
                })
                .map(|(index, _)| index)
                .collect();
            if blocked.is_empty() {
                break;
            }
            for index in blocked {
                if let Some(ren) = folder.rows.get_mut(index) {
                    ren.live = false;
                    errors.insert(index, already(&ren.target));
                }
            }
        }
        for (index, error) in &errors {
            if let Some(ren) = folder.rows.get(*index) {
                self.fail_row(
                    ren.id,
                    Action::Rename,
                    ren.kind,
                    Some(ren.source.clone()),
                    ren.target.clone(),
                    0,
                    error.clone(),
                    Some(side),
                );
            }
        }
        // Chains and cycles: each name has one mover in and one mover out.
        let by_from: BTreeMap<&str, usize> = folder
            .rows
            .iter()
            .enumerate()
            .filter(|(_, ren)| ren.live && ren.from_key != ren.to_key)
            .map(|(index, ren)| (ren.from_key.as_str(), index))
            .collect();
        let mut done = vec![false; folder.rows.len()];
        let mut order: Vec<Step> = Vec::new();
        for start in 0..folder.rows.len() {
            if done.get(start).copied().unwrap_or(true)
                || !folder.rows.get(start).is_some_and(|ren| ren.live)
            {
                continue;
            }
            // Follow what `start` waits for, down to the free end or back to `start`.
            let mut path = vec![start];
            let mut cycle = false;
            let mut at = start;
            while let Some(&next) = folder
                .rows
                .get(at)
                .and_then(|ren| by_from.get(ren.to_key.as_str()))
            {
                if next == start {
                    cycle = true;
                    break;
                }
                if done.get(next).copied().unwrap_or(true) || path.contains(&next) {
                    break;
                }
                path.push(next);
                at = next;
            }
            for index in &path {
                if let Some(slot) = done.get_mut(*index) {
                    *slot = true;
                }
            }
            if cycle {
                // `start` steps aside, each name frees the one before it, and `start` lands last.
                order.push(Step::Aside(start));
                order.extend(path.iter().skip(1).rev().map(|index| Step::Plain(*index)));
                order.push(Step::Land(start));
            } else {
                order.extend(path.iter().rev().map(|index| Step::Plain(*index)));
            }
        }
        let mut names = folder.taken.clone();
        names.extend(folder.rows.iter().map(|ren| ren.to_key.clone()));
        let mut hidden: BTreeMap<usize, Spec> = BTreeMap::new();
        for step in order {
            match step {
                Step::Plain(index) => {
                    let Some(ren) = folder.rows.get(index) else {
                        continue;
                    };
                    self.push_rename(
                        ren,
                        ren.source.clone(),
                        ren.target.clone(),
                        ren.stamped,
                        side,
                    );
                }
                Step::Aside(index) => {
                    let Some(ren) = folder.rows.get(index) else {
                        continue;
                    };
                    let Some(aside) = self.hidden_name(folder.parent.as_ref(), ren, &mut names)
                    else {
                        let error = report_with(
                            ErrorKind::InvalidInput,
                            &ren.source,
                            format!("{}: no free name to break a cycle", ren.source.canonical()),
                        );
                        self.fail_row(
                            ren.id,
                            Action::Rename,
                            ren.kind,
                            Some(ren.source.clone()),
                            ren.target.clone(),
                            0,
                            error,
                            Some(side),
                        );
                        continue;
                    };
                    self.push_rename(ren, ren.source.clone(), aside.clone(), ren.stamped, side);
                    hidden.insert(index, aside);
                }
                Step::Land(index) => {
                    let (Some(ren), Some(aside)) = (folder.rows.get(index), hidden.get(&index))
                    else {
                        continue;
                    };
                    self.push_rename(ren, aside.clone(), ren.target.clone(), None, side);
                }
            }
        }
    }

    /// One `rename` item.
    fn push_rename(&mut self, ren: &Ren, from: Spec, to: Spec, expect: Option<Expect>, side: Side) {
        let mut item = PlanItem::planned(Action::Rename, ren.kind, Some(from), to, 0);
        item.expect = expect;
        item.size = ren.size;
        let meta = ItemMeta {
            side: Some(side),
            node: Some(ren.id),
            path: self.read.path(ren.id).as_str().to_owned(),
            conflicts: Conflicts::empty(),
            answer: None,
        };
        self.push(item, meta);
    }

    /// A free hidden name: `.<name>.hikasa-ren[-n]`, or `.hikasa-ren[-n]` for a name near the limit of a file system.
    fn hidden_name(
        &mut self,
        parent: Option<&Spec>,
        ren: &Ren,
        names: &mut BTreeSet<String>,
    ) -> Option<Spec> {
        let parent = parent?;
        let fold = self.folds(parent);
        let own = ren.source.file_name()?;
        let long = format!(".{own}{HIDDEN}");
        let stem = if long.len() + 8 > NAME_MAX {
            HIDDEN.to_owned()
        } else {
            long
        };
        for number in 1..u32::MAX {
            let name = if number == 1 {
                stem.clone()
            } else {
                format!("{stem}-{number}")
            };
            if names.insert(key_of(&name, fold)) {
                let file = FileName::new(&name).ok()?;
                return self.vfs.child(parent, &file).ok();
            }
        }
        None
    }
}

/// One step of the order of a folder's renames.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// The row's rename.
    Plain(usize),
    /// The row steps aside to a hidden name.
    Aside(usize),
    /// The row lands from its hidden name.
    Land(usize),
}
