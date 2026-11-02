//! Contents on demand (FC-029 b, c; FC-040; D-174): the same engines, budget and pool as a scan's content tasks, on rows
//! of a finished comparison. Results enter the model in place, and a repeated test counts once.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use hikasa_core::{
    ContentKind, ContentMethod, ContentOutcome, ContentResult, ErrorKind, ErrorReport, Pair, Side,
    Spec,
};
use hikasa_vfs::ops::ItemStamp;
use hikasa_vfs::{Entry, FollowLinks, StreamOptions, VfsSource, crc32_source};

use super::{Scan, Task};
use crate::error::FolderError;
use crate::model::store::NONE;
use crate::model::{FolderModel, NodeId, NodeKind};
use crate::resolve::side_index;
use crate::scan::FolderEnv;
use crate::stats::ContentCounts;

/// One result of [`compare_contents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentItem {
    /// The row.
    pub node: NodeId,
    /// Its path.
    pub path: String,
    /// The result.
    pub kind: ContentKind,
}

/// What [`compare_contents`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentReport {
    /// The results of this run.
    pub counts: ContentCounts,
    /// One per pair tested.
    pub items: Vec<ContentItem>,
    /// False when cancelled: the pairs not reached keep what they had.
    pub complete: bool,
}

/// The file pairs of `nodes`: a pair itself, or a folder's pairs below it, what is hidden left out.
fn pairs_of(model: &FolderModel, nodes: &[NodeId]) -> Vec<(NodeId, Spec, Spec)> {
    let read = model.read();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<NodeId> = nodes.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let Some(view) = read.node(id) else { continue };
        if view.is_folder() {
            if !view.left.is_some_and(|e| e.filtered) || !view.right.is_some_and(|e| e.filtered) {
                let mut children: Vec<NodeId> = read.children(id).collect();
                children.reverse();
                stack.extend(children);
            }
            continue;
        }
        let (Some(left), Some(right)) = (view.left, view.right) else {
            continue;
        };
        if view.kind != NodeKind::File || left.filtered || right.filtered || !seen.insert(id) {
            continue;
        }
        if let (Some(ls), Some(rs)) = (read.spec(id, Side::Left), read.spec(id, Side::Right)) {
            out.push((id, ls, rs));
        }
    }
    out
}

/// The entries of a pair; a failure is what a failed read of it would be.
fn stat_pair(
    env: &FolderEnv<'_>,
    follow: FollowLinks,
    left: &Spec,
    right: &Spec,
) -> Result<(Entry, Entry), ErrorReport> {
    let stat = |spec: &Spec| {
        env.vfs
            .stat(spec, follow)
            .map_err(|e| ErrorReport::capture(&e, Some(spec)))
    };
    Ok((stat(left)?, stat(right)?))
}

/// FC-029(b): the content test `method` on the file pairs of `nodes` (a folder: its pairs below it, what filters hide
/// left out). Results enter the model in place. Runs on `env.pool` with the shared budget (D-25), as a scan's content
/// tasks. A second call while one runs, or a refresh, → [`FolderError::RefreshBusy`].
pub fn compare_contents(
    model: &Arc<FolderModel>,
    nodes: &[NodeId],
    method: ContentMethod,
    env: &FolderEnv<'_>,
) -> Result<ContentReport, FolderError> {
    if method == ContentMethod::None {
        return Err(FolderError::InvalidRules(
            "a content test needs a method".to_owned(),
        ));
    }
    if env.content.get(method).is_none() {
        return Err(FolderError::RulesComparatorMissing);
    }
    let _claim = model.claim_refresh()?;
    let mut scan = Scan::for_model(env, model)?;
    scan.content_method = method;
    let pairs = pairs_of(model, nodes);
    let ids: Vec<NodeId> = pairs.iter().map(|(id, _, _)| *id).collect();
    model.mutate(|s| s.set_pending(&ids, true));
    let mut tasks: Vec<Task> = Vec::with_capacity(pairs.len());
    for (id, left, right) in pairs {
        match stat_pair(env, scan.follow, &left, &right) {
            Ok((le, re)) => tasks.push(scan.content_of(id, Box::new([(left, le), (right, re)]))),
            Err(error) => {
                let outcome = ContentOutcome {
                    method,
                    result: ContentResult::Error { error },
                    warnings: Vec::new(),
                };
                model.mutate(|s| s.apply_content(id, outcome));
            }
        }
    }
    scan.execute(tasks);
    scan.flush();
    let complete = !scan.aborted.load(Ordering::Acquire);
    // What a stop left without an outcome is as it was.
    model.mutate(|s| s.set_pending(&ids, false));
    let read = model.read();
    let mut report = ContentReport {
        complete,
        ..ContentReport::default()
    };
    for id in ids {
        let Some(summary) = read.node(id).and_then(|view| view.content) else {
            continue;
        };
        if summary.method != method {
            continue;
        }
        report.counts.add(summary.kind);
        report.items.push(ContentItem {
            node: id,
            path: read.path(id).as_str().to_owned(),
            kind: summary.kind,
        });
    }
    Ok(report)
}

/// FC-040: CRC-32 of the given sides: `Entry::crc32` where a back-end gives one (D-147), else read. Each result goes to
/// the model's side table, so `EntryView::crc32` shows it; a refresh drops it with a changed row. A stop leaves the rest
/// with the error `cancelled`.
pub fn compute_crcs(
    model: &Arc<FolderModel>,
    items: &[(NodeId, Side)],
    env: &FolderEnv<'_>,
) -> Result<Vec<(NodeId, Side, Result<u32, ErrorReport>)>, FolderError> {
    let _claim = model.claim_refresh()?;
    let follow = if model.rules().handling.follow_symlinks {
        FollowLinks::Yes
    } else {
        FollowLinks::No
    };
    let mut out = Vec::with_capacity(items.len());
    for &(id, side) in items {
        let known = {
            let read = model.read();
            let entry = read.node(id).and_then(|view| view.side(side).copied());
            entry.map(|entry| (entry.crc32, read.spec(id, side)))
        };
        let result = match known {
            None => Err(missing(id)),
            Some((Some(crc), _)) => Ok(crc),
            Some((None, None)) => Err(missing(id)),
            Some((None, Some(spec))) => crc_of(env, follow, spec),
        };
        if let Ok(crc) = result {
            model.mutate(|s| s.set_crc(id, side_index(side), crc));
        }
        out.push((id, side, result));
    }
    Ok(out)
}

/// The report of a side that is not there.
fn missing(id: NodeId) -> ErrorReport {
    ErrorReport {
        kind: ErrorKind::NotFound,
        message: format!("row {} has no such side", id.0),
        spec: None,
        causes: Vec::new(),
    }
}

/// The CRC-32 of one file.
fn crc_of(env: &FolderEnv<'_>, follow: FollowLinks, spec: Spec) -> Result<u32, ErrorReport> {
    let report = |e: &hikasa_vfs::VfsError, spec: &Spec| ErrorReport::capture(e, Some(spec));
    let entry = env.vfs.stat(&spec, follow).map_err(|e| report(&e, &spec))?;
    if let Some(crc) = entry.crc32 {
        return Ok(crc);
    }
    let cancel = env.ctl.cancel;
    let source = VfsSource::new(env.vfs, spec.clone(), entry).with_cancel(cancel);
    crc32_source(&source, &StreamOptions::default(), &env.ctl).map_err(|e| report(&e, &spec))
}

impl FolderModel {
    /// FC-029(c): the result of a child view for its pair. Applied only while both sides still have `stamps` (size and
    /// time); `false` otherwise. A result the row already has is replaced.
    pub fn apply_child_result(
        &self,
        node: NodeId,
        stamps: Pair<ItemStamp>,
        outcome: ContentOutcome,
    ) -> bool {
        let holds = {
            let read = self.read();
            read.node(node).is_some_and(|view| {
                let same = |entry: Option<&crate::model::EntryView<'_>>, stamp: &ItemStamp| {
                    entry.is_some_and(|entry| {
                        stamp.size.is_none_or(|size| size == entry.size)
                            && stamp.mtime.is_none_or(|time| entry.mtime == Some(time))
                    })
                };
                view.parent.map_or(0, |p| p.0) != NONE
                    && same(view.left.as_ref(), &stamps.left)
                    && same(view.right.as_ref(), &stamps.right)
            })
        };
        if holds {
            self.mutate(|s| s.apply_content(node, outcome));
        }
        holds
    }
}
