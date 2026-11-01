//! The batches of one operation (M2 W5b, D-148; §1.5.10): `execute` is where an operation ends, so it is the one
//! place that opens them and finishes them.
//!
//! A root is one outermost file on one side of the operation. Before an item touches a spec inside an archive the
//! root's batch is opened, the first time. At the end: every target root, then the second phase of a move, then the
//! source-only roots. A root that is aborted, or whose rebuild failed, takes its done items with it, and [`Roots`] going
//! away with a batch still open aborts it, so a panic inside `execute` cannot leave one behind.

use std::cell::Cell;

use hikasa_core::{Ctl, ErrorReport, Spec};

use super::plan::report;
use super::{Action, Exec, ItemResult, ItemStatus, PlanItem, same_instance};
use crate::error::VfsError;
use crate::vfs::{Finish, Vfs};

/// Which back-end a root is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Src,
    Dst,
}

/// One archive file the operation changes.
struct Root {
    side: Side,
    /// The archive the batch was opened with.
    at: Spec,
    /// The file outside every archive: what makes two chains one batch.
    outer: Spec,
    /// An item writes to it, so it finishes before the roots that only lose sources.
    target: bool,
    open: Cell<bool>,
}

/// The roots an item belongs to.
type Owned = Option<(Option<usize>, Option<usize>)>;

/// The batches of one `execute`.
pub(super) struct Roots<'a> {
    src: &'a dyn Vfs,
    dst: &'a dyn Vfs,
    roots: Vec<Root>,
    /// The roots of each result, in order.
    owned: Vec<Owned>,
}

impl Drop for Roots<'_> {
    fn drop(&mut self) {
        for root in &self.roots {
            if root.open.get() {
                let _ = self.vfs(root.side).finish(&root.at, Finish::Abort);
            }
        }
    }
}

impl<'a> Roots<'a> {
    pub(super) fn new(src: &'a dyn Vfs, dst: &'a dyn Vfs) -> Roots<'a> {
        Roots {
            src,
            dst,
            roots: Vec::new(),
            owned: Vec::new(),
        }
    }

    fn vfs(&self, side: Side) -> &'a dyn Vfs {
        match side {
            Side::Src => self.src,
            Side::Dst => self.dst,
        }
    }

    /// The root of `spec`, opened the first time; `None` outside archives.
    fn open(&mut self, side: Side, spec: &Spec, target: bool) -> Result<Option<usize>, VfsError> {
        let (Some(at), Some(outer)) = (spec.container(), spec.outermost_file()) else {
            return Ok(None);
        };
        // One instance on both sides is one batch.
        let same = |root: &Root| {
            root.outer == *outer && (root.side == side || same_instance(self.src, self.dst))
        };
        if let Some(found) = self.roots.iter().position(same) {
            if target && let Some(root) = self.roots.get_mut(found) {
                root.target = true;
            }
            return Ok(Some(found));
        }
        self.vfs(side).begin(at)?;
        self.roots.push(Root {
            side,
            at: at.clone(),
            outer: outer.clone(),
            target,
            open: Cell::new(true),
        });
        Ok(Some(self.roots.len() - 1))
    }

    /// Opens what `item` is about to touch: its target on `dst`, a source on `src` for moves and removes.
    pub(super) fn touch(&mut self, item: &PlanItem) -> Result<Owned, VfsError> {
        let (target, source) = match item.action {
            Action::Remove | Action::RemoveDir | Action::Trash => {
                (None, self.open(Side::Src, &item.target, false)?)
            }
            Action::Move | Action::Rename => {
                let source = match &item.source {
                    Some(source) => self.open(Side::Src, source, false)?,
                    None => None,
                };
                (self.open(Side::Dst, &item.target, true)?, source)
            }
            Action::Skip => (None, None),
            _ => (self.open(Side::Dst, &item.target, true)?, None),
        };
        Ok((target.is_some() || source.is_some()).then_some((target, source)))
    }

    /// Records the roots of the result that follows.
    pub(super) fn note(&mut self, owned: Owned) {
        self.owned.push(owned);
    }

    /// Results noted so far.
    pub(super) fn noted(&self) -> usize {
        self.owned.len()
    }

    fn belongs(&self, result: usize, root: usize) -> bool {
        self.owned
            .get(result)
            .copied()
            .flatten()
            .is_some_and(|(target, source)| target == Some(root) || source == Some(root))
    }

    /// Ends one root. `Ok(())` when the rebuild went through.
    fn end(&self, root: usize, how: Finish, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        let Some(found) = self.roots.get(root) else {
            return Ok(());
        };
        found.open.set(false);
        self.vfs(found.side).finish_with(&found.at, how, ctl)
    }

    /// The done items of `root` lost their rebuild.
    fn unwind(&self, root: usize, results: &mut [ItemResult], error: Option<&ErrorReport>) {
        for (index, result) in results.iter_mut().enumerate() {
            if result.status == ItemStatus::Done && self.belongs(index, root) {
                *result = match error {
                    Some(error) => ItemResult::failed(error.clone()),
                    None => ItemResult::status(ItemStatus::NotRun),
                };
            }
        }
    }
}

/// Finishes every root (§1.5.10 steps 3 and 4).
pub(super) fn finish(
    roots: &mut Roots<'_>,
    exec: &mut Exec<'_>,
    items: &[PlanItem],
    results: &mut [ItemResult],
    cancelled: &mut bool,
) {
    let ctl = exec.ctl;
    let order: Vec<usize> = (0..roots.roots.len()).collect();
    let (targets, sources): (Vec<usize>, Vec<usize>) = order
        .into_iter()
        .partition(|&at| roots.roots.get(at).is_some_and(|root| root.target));
    let mut failure: Option<ErrorReport> = None;
    let mut lost = Vec::new();
    for root in targets {
        let how = if *cancelled {
            Finish::Abort
        } else {
            Finish::Commit
        };
        match (how, roots.end(root, how, ctl)) {
            (Finish::Abort, _) => {
                lost.push(root);
                roots.unwind(root, results, None);
            }
            (Finish::Commit, Ok(())) => {}
            (Finish::Commit, Err(e)) if e.is_cancelled() => {
                *cancelled = true;
                lost.push(root);
                roots.unwind(root, results, None);
            }
            (Finish::Commit, Err(e)) => {
                let error = report(&e);
                lost.push(root);
                roots.unwind(root, results, Some(&error));
                failure.get_or_insert(error);
            }
        }
    }
    if *cancelled {
        // A rebuild went through before the stop: its copies stay, and a move that kept its source is not done.
        for (index, _) in std::mem::take(&mut exec.deferred) {
            if let Some(result) = results.get_mut(index) {
                *result = ItemResult::status(ItemStatus::NotRun);
            }
        }
    } else {
        // A failed rebuild leaves every source in place: a folder stays even with none of its own moved.
        let whole = lost.is_empty();
        exec.run_deferred(items, results, &|index, moved| {
            whole || (moved && !lost.iter().any(|&root| roots.belongs(index, root)))
        });
    }
    for root in sources {
        let how = if *cancelled || failure.is_some() {
            Finish::Abort
        } else {
            Finish::Commit
        };
        match (how, roots.end(root, how, ctl)) {
            (Finish::Abort, _) => roots.unwind(root, results, failure.as_ref()),
            (Finish::Commit, Ok(())) => {}
            (Finish::Commit, Err(e)) if e.is_cancelled() => {
                *cancelled = true;
                roots.unwind(root, results, None);
            }
            (Finish::Commit, Err(e)) => roots.unwind(root, results, Some(&report(&e))),
        }
    }
}
