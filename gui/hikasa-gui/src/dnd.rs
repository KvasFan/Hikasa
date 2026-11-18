//! Drag and drop (M2 W4a, UI-008; M2 W9c, FC-080, D-251): the targets and the rule of a drop from the OS ([`decide`])
//! and of rows dragged inside a folder view ([`decide_rows`]); the app core applies them
//! ([`crate::app::AppCore::drop_paths`], [`crate::app::AppCore::drop_rows`]). The UI layer reads `raw.dropped_files`
//! (paths only; bytes-only drops are ignored) and highlights the target under `raw.hovered_files` or under egui's own
//! drag payload; delivery from the OS depends on the platform backend: winit 0.30 delivers drops on X11, Windows and
//! macOS, not on Wayland, and cannot start a drag toward other programs (the clipboard's file lists stand in, D-252).

use std::path::PathBuf;

use hikasa_core::Side;
use hikasa_folder::NodeId;
use hikasa_folder::select::Selection;
use hikasa_session::sessions::SessionType;

/// The message of a drop the window cannot use.
pub const DROP_ONE_OR_TWO: &str = "Drop one or two files or folders.";
/// The hint of a drop of one path on a window.
pub const DROP_ONE_MORE: &str =
    "Drop two files or two folders to compare them, or drop one on a path field.";

/// A path field that takes drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldId {
    /// Home: the left path.
    HomeLeft,
    /// Home: the right path.
    HomeRight,
}

/// Where paths were dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropTarget {
    /// A path field.
    PathField(FieldId),
    /// A type button of Home (W4b).
    HomeType(SessionType),
    /// A folder view's path edit or the header of its pane (M2 W9c): the side's base.
    PanePath(Side),
    /// A folder view's tree (M2 W9c): a folder row of the side (`Some`), or the empty area, the side's base (`None`).
    PaneTree {
        /// The pane.
        side: Side,
        /// The folder row under the pointer.
        folder: Option<NodeId>,
    },
    /// A folder view, the place unknown (W9c review F4): winit gives no pointer position during a drag from another
    /// program (X11, Windows, macOS), and a position older than the drag is not used; the prompt asks for the pane.
    PaneAsk,
    /// A picture view's pane or header (W13b review finding 17, §1.13.8): the side it replaces.
    PictureSide(Side),
    /// The window (any view other than Home's targets).
    Window,
}

/// What a drop does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropOutcome {
    /// Fills path fields.
    Fill(Vec<(FieldId, String)>),
    /// Opens a comparison of two paths like the command line.
    Open([PathBuf; 2]),
    /// Shows a message in the view's message panel.
    Message(&'static str),
    /// A Home type button took one path: a new session of that type with the left side filled.
    NewSession(SessionType, String),
    /// A Home type button took two paths: that type's view.
    OpenType(SessionType, [PathBuf; 2]),
    /// A folder view's bases (M2 W9c): the path edits of these sides take these texts, then Accept.
    Bases(Vec<(Side, String)>),
    /// A folder view's pane took paths (M2 W9c): the prompt "Copy (or move) `<n>` items into `<folder>`?" with the plan
    /// of a copy and of a move into that folder (`vfs_op_plan`), then the plan chosen runs as it is.
    Transfer {
        /// The pane.
        side: Side,
        /// The folder row (`None`: the side's base).
        folder: Option<NodeId>,
        /// The paths, as dropped.
        paths: Vec<PathBuf>,
    },
    /// Paths dropped on a folder view where the window could not say which pane (W9c review F4): the prompt asks
    /// which pane's current folder they go into and, for one or two paths, offers W4's window rule (*Open as new
    /// comparison*).
    Ask(Vec<PathBuf>),
    /// A picture view's sides replaced (reloads).
    Reopen(Vec<(Side, String)>),
    /// Nothing (no path).
    Ignore,
}

/// The rule of a drop: a path field takes the first path (two paths on the left field: left and right); the
/// window opens two paths, hints at one and refuses more; a Home type button takes one path into a new session
/// of its type and opens two as that type.
pub fn decide(target: DropTarget, paths: &[PathBuf]) -> DropOutcome {
    let text = |p: &PathBuf| p.to_string_lossy().into_owned();
    match (target, paths) {
        (_, []) => DropOutcome::Ignore,
        (DropTarget::PathField(FieldId::HomeLeft), [l, r, ..]) => DropOutcome::Fill(vec![
            (FieldId::HomeLeft, text(l)),
            (FieldId::HomeRight, text(r)),
        ]),
        (DropTarget::PathField(field), [first, ..]) => {
            DropOutcome::Fill(vec![(field, text(first))])
        }
        (DropTarget::HomeType(kind), [one]) => DropOutcome::NewSession(kind, text(one)),
        (DropTarget::HomeType(kind), [l, r]) => DropOutcome::OpenType(kind, [l.clone(), r.clone()]),
        (DropTarget::PanePath(Side::Left), [l, r, ..]) => {
            DropOutcome::Bases(vec![(Side::Left, text(l)), (Side::Right, text(r))])
        }
        (DropTarget::PanePath(side), [first, ..]) => DropOutcome::Bases(vec![(side, text(first))]),
        (DropTarget::PaneTree { side, folder }, paths) => DropOutcome::Transfer {
            side,
            folder,
            paths: paths.to_vec(),
        },
        (DropTarget::PaneAsk, paths) => DropOutcome::Ask(paths.to_vec()),
        // A picture view (§1.13.8): one file replaces the side dropped on, two replace both.
        (DropTarget::PictureSide(side), [one]) => DropOutcome::Reopen(vec![(side, text(one))]),
        (DropTarget::PictureSide(_), [l, r]) => {
            DropOutcome::Reopen(vec![(Side::Left, text(l)), (Side::Right, text(r))])
        }
        (DropTarget::PictureSide(_), _) => DropOutcome::Message(DROP_ONE_OR_TWO),
        (DropTarget::Window, [_]) => DropOutcome::Message(DROP_ONE_MORE),
        (DropTarget::Window, [l, r]) => DropOutcome::Open([l.clone(), r.clone()]),
        (DropTarget::Window | DropTarget::HomeType(_), _) => DropOutcome::Message(DROP_ONE_OR_TWO),
    }
}

/// Rows dragged in a folder view (D-251): the side they were pressed on and the selection they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDrag {
    /// The pane the drag started on (the centre: `None`, both sides).
    pub from: Option<Side>,
    /// The dragged rows (the selection at the press).
    pub rows: Selection,
}

/// Where dragged rows are released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTarget {
    /// A pane of the same view: a folder row of that side (`Some`), or anywhere else in it (`None`).
    Pane {
        /// The pane.
        side: Side,
        /// The folder row under the pointer.
        folder: Option<NodeId>,
    },
    /// The centre column, a header, another view, another window, outside the window.
    Elsewhere,
}

/// What a release of dragged rows does (§1.9.16's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowDrop {
    /// `CopySide` (`MoveSide` when `moving`) of the rows from `from`, through D-244's flow.
    ToOtherSide {
        /// The side the rows are copied from.
        from: Side,
        /// Shift held at the release.
        moving: bool,
    },
    /// `CopyTo` (`MoveTo` when `moving`) of the rows from `from` into the folder row, structure *relative*, the dialog
    /// prefilled (its preview shows first).
    IntoFolder {
        /// The side the rows are copied from.
        from: Side,
        /// The folder row of the other pane.
        folder: NodeId,
        /// Shift held at the release.
        moving: bool,
    },
    /// Nothing: the own pane, the centre, out of the window, nothing dragged (D-251's limits).
    Nothing,
}

/// The rule of a release of dragged rows: on the other pane outside a folder row they go to the other side, on a folder
/// row of the other pane into that folder; Shift moves; the own pane, the centre and everything outside the view do
/// nothing. Rows dragged from the centre (both sides) come from the side opposite the release.
pub fn decide_rows(drag: &RowDrag, target: RowTarget, shift: bool) -> RowDrop {
    let RowTarget::Pane { side, folder } = target else {
        return RowDrop::Nothing;
    };
    if drag.rows.is_empty() || !matches!(side, Side::Left | Side::Right) {
        return RowDrop::Nothing;
    }
    let from = side.opposite();
    if drag.from.is_some_and(|f| f != from) {
        return RowDrop::Nothing;
    }
    match folder {
        Some(folder) => RowDrop::IntoFolder {
            from,
            folder,
            moving: shift,
        },
        None => RowDrop::ToOtherSide {
            from,
            moving: shift,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_follow_the_target() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(decide(DropTarget::Window, &[]), DropOutcome::Ignore);
        assert_eq!(
            decide(
                DropTarget::PathField(FieldId::HomeRight),
                &[p("/a"), p("/b")]
            ),
            DropOutcome::Fill(vec![(FieldId::HomeRight, "/a".into())])
        );
        assert_eq!(
            decide(
                DropTarget::PathField(FieldId::HomeLeft),
                &[p("/a"), p("/b")]
            ),
            DropOutcome::Fill(vec![
                (FieldId::HomeLeft, "/a".into()),
                (FieldId::HomeRight, "/b".into())
            ])
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a")]),
            DropOutcome::Message(DROP_ONE_MORE)
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a"), p("/b")]),
            DropOutcome::Open([p("/a"), p("/b")])
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a"), p("/b"), p("/c")]),
            DropOutcome::Message(DROP_ONE_OR_TWO)
        );
        let kind = SessionType::TextCompare;
        assert_eq!(
            decide(DropTarget::HomeType(kind), &[p("/a")]),
            DropOutcome::NewSession(kind, "/a".into())
        );
        assert_eq!(
            decide(DropTarget::HomeType(kind), &[p("/a"), p("/b")]),
            DropOutcome::OpenType(kind, [p("/a"), p("/b")])
        );
        assert_eq!(
            decide(DropTarget::HomeType(kind), &[p("/a"), p("/b"), p("/c")]),
            DropOutcome::Message(DROP_ONE_OR_TWO)
        );
    }
}
