//! What a folder view's panes add in M2 W9c (§1.9.16–§1.9.18): the items `Copy` puts on the clipboard as files and the
//! specs `Copy Filename` writes as text (D-252), the folder a drop or a paste goes into and the run of the plan its
//! prompt answered (D-251), the item of the file-manager entries (D-254), and the right-click that selects first
//! (D-253). Toolkit-free (check 7): core gives every spec, plan and line.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use hikasa_core::{JobKind, Side, Spec};
use hikasa_folder::NodeId;
use hikasa_folder::select::{SideSet, items_in_order};
use hikasa_session::Service;
use hikasa_session::api::spec_os_string;
use hikasa_session::ipc::ViewId;

use super::{FolderViewModel, Mark, Task};
use crate::drop_dialog::{DropAnswer, DropInto};
use crate::folder::selection::{Hit, Mods, Press};
use crate::job::Job;

/// The message of `Copy` when items are not on a local disk (D-252).
pub const NOT_LOCAL: &str =
    "Only items on local disks go to the clipboard as files; Copy Filename copies every path.";

/// The message of a paste with both sides focused (no pane to paste into).
pub const PASTE_WHERE: &str = "Select a row on the side to paste into.";

/// The caption of a drop's run.
fn transfer_caption(moving: bool) -> &'static str {
    if moving {
        "Move dropped items"
    } else {
        "Copy dropped items"
    }
}

/// The name of a side in messages.
fn side_name(side: Side) -> &'static str {
    if side == Side::Right { "right" } else { "left" }
}

impl FolderViewModel {
    /// `Copy` (D-252): the local paths of the selected items on `side` (else the focus sides), in tree order, left
    /// side first per row; and how many selected items are not on a local disk (members, other back-ends), left out.
    pub fn clipboard_paths(&self, side: Option<Side>) -> (Vec<PathBuf>, usize) {
        let Some(prepared) = self.prepared() else {
            return (Vec::new(), 0);
        };
        let sides = match side {
            Some(side @ (Side::Left | Side::Right)) => SideSet::of(side),
            _ => self.selection.focus,
        };
        let mut items = self.selection.items.clone();
        items.remove(NodeId::ROOT, SideSet::all());
        let read = prepared.model().read();
        let mut paths = Vec::new();
        let mut left_out = 0;
        for (node, item_side) in items_in_order(&read, &items) {
            if !sides.contains(SideSet::of(item_side)) {
                continue;
            }
            match read.spec(node, item_side) {
                Some(Spec::Local(path)) => paths.push(path.as_path().to_path_buf()),
                Some(_) => left_out += 1,
                None => {}
            }
        }
        (paths, left_out)
    }

    /// `Copy Filename` (FC-063): the selected items' specs (`Spec::canonical`), one a line, in tree order, left side
    /// first per row; `\r\n` between lines on Windows (`windows`), else `\n`; no line end after the last.
    pub fn filenames_text(&self, windows: bool) -> String {
        let Some(prepared) = self.prepared() else {
            return String::new();
        };
        let mut items = self.selection.items.clone();
        items.remove(NodeId::ROOT, SideSet::all());
        let read = prepared.model().read();
        let lines: Vec<String> = items_in_order(&read, &items)
            .into_iter()
            .filter_map(|(node, side)| read.spec(node, side).map(|s| s.canonical()))
            .collect();
        lines.join(if windows { "\r\n" } else { "\n" })
    }

    /// The folder of `side` a drop on `folder` (a folder row; `None`: the empty area) goes into (D-251): that folder,
    /// or its nearest folder above that the side has, else the side's base; `Err` with the message when the side may
    /// not be written (*Disable editing*, `--ro`) or the view has no comparison.
    pub fn drop_into(
        &self,
        view: ViewId,
        side: Side,
        folder: Option<NodeId>,
    ) -> Result<DropInto, String> {
        let prepared = self
            .prepared()
            .ok_or_else(|| "The view has no comparison yet.".to_owned())?;
        if !matches!(side, Side::Left | Side::Right) {
            return Err(PASTE_WHERE.to_owned());
        }
        if !self.write.writable(side) {
            return Err(format!(
                "The {} side is read-only (Disable editing).",
                side_name(side)
            ));
        }
        let read = prepared.model().read();
        let mut at = folder.unwrap_or(NodeId::ROOT);
        let spec = loop {
            let here = read
                .node(at)
                .filter(|n| n.is_folder() || at == NodeId::ROOT)
                .and_then(|_| read.spec(at, side))
                .filter(|_| present_on(&read, at, side));
            match (here, read.node(at).and_then(|n| n.parent)) {
                (Some(spec), _) => break spec,
                (None, Some(parent)) => at = parent,
                (None, None) => {
                    return Err(format!("The {} side has no base folder.", side_name(side)));
                }
            }
        };
        let shown = spec.canonical();
        let mut target = spec_os_string(&spec);
        let separator = if matches!(spec, Spec::Local(_)) {
            std::path::MAIN_SEPARATOR_STR
        } else {
            "/"
        };
        let ends = target
            .to_string_lossy()
            .ends_with(['/', std::path::MAIN_SEPARATOR]);
        if !ends {
            target.push(separator);
        }
        Ok(DropInto {
            view,
            side,
            folder: (at != NodeId::ROOT).then_some(at),
            shown,
            target,
        })
    }

    /// `Paste` (D-252): the pane's current folder: the current row on `side` (else the focus side) when it is a folder
    /// there, else its folder, else the base. With both sides focused and no side given there is no pane.
    pub fn paste_into(&self, view: ViewId, side: Option<Side>) -> Result<DropInto, String> {
        let side = match side {
            Some(side @ (Side::Left | Side::Right)) => side,
            _ if self.selection.focus == SideSet::LEFT => Side::Left,
            _ if self.selection.focus == SideSet::RIGHT => Side::Right,
            _ => return Err(PASTE_WHERE.to_owned()),
        };
        let folder = self.selection.current.and_then(|node| {
            let prepared = self.prepared()?;
            let read = prepared.model().read();
            let row = read.node(node)?;
            if row.is_folder() {
                Some(node)
            } else {
                row.parent.filter(|p| *p != NodeId::ROOT)
            }
        });
        self.drop_into(view, side, folder)
    }

    /// The item of the file-manager entries (D-254): the current row on `side` (else the focus side, else the side the
    /// current row has, the left first), its spec and whether it is a folder.
    pub fn explorer_item(&self, side: Option<Side>) -> Option<(Spec, bool)> {
        let prepared = self.prepared()?;
        let node = self.selection.current?;
        let read = prepared.model().read();
        let row = read.node(node)?;
        let sides: Vec<Side> = match side {
            Some(side @ (Side::Left | Side::Right)) => vec![side],
            _ if self.selection.focus == SideSet::LEFT => vec![Side::Left],
            _ if self.selection.focus == SideSet::RIGHT => vec![Side::Right],
            _ => vec![Side::Left, Side::Right],
        };
        sides
            .into_iter()
            .filter(|s| present_on(&read, node, *s))
            .find_map(|s| read.spec(node, s))
            .map(|spec| (spec, row.is_folder()))
    }

    /// Per side: the current row's item there has file-manager targets (a local item or a member of a local archive).
    pub fn explorer_sides(&self) -> [bool; 2] {
        [Side::Left, Side::Right].map(|side| {
            self.explorer_item(Some(side))
                .is_some_and(|(spec, folder)| {
                    hikasa_session::api::file_manager_targets(&spec, folder).is_some()
                })
        })
    }

    /// A right-click on row `index` at `hit` (D-253): a row outside the selection (on that side) becomes the selection
    /// first, as a click; a selected row keeps the selection.
    pub fn context_press(&mut self, index: usize, hit: Hit) {
        if self.selection_press(index, hit) == Press::Swipe {
            self.click(index, hit, Mods::default());
        }
    }

    /// The folder row of `side` at visible row `index` (a drop target, D-251): the row when it is a folder.
    pub fn folder_row(&self, index: usize) -> Option<NodeId> {
        let row = self.rows.get(index)?;
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        read.node(row.node)
            .is_some_and(|n| n.is_folder())
            .then_some(row.node)
    }

    /// The drop's prompt opens (D-251): the view is busy with it until it answers. False (the window says "Wait for
    /// the current operation to end.") while another task runs or the comparison does.
    pub fn start_drop_prompt(&mut self) -> bool {
        if !self.idle() || self.busy() {
            return false;
        }
        let Some(mark) = self.prepared().map(Mark::of) else {
            return false;
        };
        self.cancel_crc();
        self.task = Some(Task::Dialog {
            caption: "Drop".to_owned(),
            mark,
            items: hikasa_folder::select::Selection::default(),
        });
        true
    }

    /// The drop's prompt closed: the plan of the button pressed runs as it is (`vfs_op_execute`, T12); after it, the
    /// folder it went into and, for a move, the folders the sources left that this comparison shows are refreshed.
    pub fn drop_answered(&mut self, service: &Service, answer: Option<DropAnswer>) {
        if !matches!(self.task, Some(Task::Dialog { .. })) {
            return;
        }
        self.task = None;
        let Some(answer) = answer else {
            return;
        };
        let caption = transfer_caption(answer.moving).to_owned();
        let mut refresh = vec![answer.into.folder.unwrap_or(NodeId::ROOT)];
        if answer.moving {
            refresh.extend(self.nodes_of_parents(&answer.sources));
        }
        refresh.sort_unstable_by_key(|n| n.0);
        refresh.dedup();
        #[cfg(test)]
        {
            self.executed = Some(serde_json::to_string(&answer.plan).expect("a plan serializes"));
        }
        let crc = self.crc_yield();
        #[cfg(test)]
        let gate = self.gate.clone();
        let plan = answer.plan;
        let job = Job::spawn(
            service,
            JobKind::VfsWrite,
            caption.clone(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                super::wait_at(gate.as_ref());
                crc.wait();
                svc.vfs_op_execute(plan, job)
            },
        );
        let mark = self.prepared().map(Mark::of);
        self.task = Some(Task::Transfer {
            caption,
            moving: answer.moving,
            mark,
            refresh,
            began: Instant::now(),
            job,
        });
    }

    /// The folders of the view that hold `sources` (local paths below a local base), by the comparison's paths.
    fn nodes_of_parents(&self, sources: &[OsString]) -> Vec<NodeId> {
        let Some(prepared) = self.prepared() else {
            return Vec::new();
        };
        let read = prepared.model().read();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            let Some(Spec::Local(base)) = prepared.specs().get(side) else {
                continue;
            };
            for source in sources {
                let path = PathBuf::from(source);
                let Some(parent) = path.parent() else {
                    continue;
                };
                let Ok(rel) = parent.strip_prefix(base.as_path()) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let node = if rel.is_empty() {
                    Some(NodeId::ROOT)
                } else {
                    super::live_at(&read, &rel, true)
                };
                out.extend(node);
            }
        }
        out
    }
}

/// True when `node` has an item on `side` (the base always).
fn present_on(read: &hikasa_folder::TreeRead<'_>, node: NodeId, side: Side) -> bool {
    node == NodeId::ROOT || super::present_sides(read, node).contains(SideSet::of(side))
}
